//! Guard for daimon (Phase 5).
//!
//! The Guard sits between broker.execute and transport dispatch. It owns
//! three concerns:
//!
//! - **KillSwitch** (D13) — file watcher + SIGUSR1 handler. When engaged,
//!   every broker.execute is denied. Resume requires `rm` of the file —
//!   no auto-resume, no agent override.
//! - **Policy DSL** — Rust + TOML, type-checked at load. Rules: allow,
//!   deny, require_approval, optional time-of-day / blast-radius
//!   thresholds.
//! - **ApprovalQueue** — Postgres-backed inbox. broker.execute on a
//!   require_approval capability blocks until an operator approves
//!   (with a timeout that defaults to denied).
//!
//! Per the masterplan §4.6 and the practitioner-AI-Guard incident (Post 2
//! reference) this kill-switch posture is operator-only and not
//! agent-overridable by design.

pub mod approvals;
pub mod blast_radius;
pub mod error;
pub mod kill_switch;
pub mod policy;

pub use approvals::{ApprovalQueue, ApprovalRecord, ApprovalStatus};
pub use blast_radius::{
    blast_radius_for_target, enrich_with_blast_radius, ApprovalWithBlastRadius,
    DEFAULT_BLAST_RADIUS_DEPTH,
};
pub use error::{Error, Result};
pub use kill_switch::{KillState, KillSwitch};
pub use policy::{Decision, PolicyEngine, PolicyRule, PolicyVerdict};

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Compiled default for the approval wait before an unattended require_approval
/// capability is denied (P6 FR-CFG-06; overridable live from `app_config`).
pub const DEFAULT_APPROVAL_TIMEOUT_SECS: u64 = 300;

/// Guard facade — the single struct the broker holds for kill / policy /
/// approval checks. Cheap to clone (Arc-internal).
#[derive(Clone)]
pub struct Guard {
    kill: KillState,
    policy: Arc<PolicyEngine>,
    approvals: Arc<ApprovalQueue>,
    /// How long broker.execute will wait for an operator approval before
    /// failing. Held as a live `Arc<AtomicU64>` (seconds) so an operator edit
    /// in `/settings` (guard.approval_timeout_secs) applies to the NEXT gated
    /// request with no restart (P6 FR-CFG-06). Cloned handles share the atomic.
    approval_timeout_secs: Arc<AtomicU64>,
    /// P7-1 (FR-GW-13): optional bus handle. When a write parks for approval,
    /// the guard publishes an `awaiting_approval` envelope so the alert router
    /// can notify an approver on their channel. Abstract `BusHandle` only (the
    /// same seam the observer uses) — never the concrete InProcBus. Set once at
    /// boot via [`set_bus`](Self::set_bus) after the bus exists; `None` (never
    /// set) keeps the pre-P7 behaviour: park silently, console-only.
    bus: std::sync::OnceLock<Arc<dyn daimon_core::BusHandle>>,
}

impl Guard {
    pub fn new(kill: KillState, policy: PolicyEngine, approvals: ApprovalQueue) -> Self {
        Self {
            kill,
            policy: Arc::new(policy),
            approvals: Arc::new(approvals),
            approval_timeout_secs: Arc::new(AtomicU64::new(DEFAULT_APPROVAL_TIMEOUT_SECS)),
            bus: std::sync::OnceLock::new(),
        }
    }

    /// P7-1 (FR-GW-13): wire the bus handle (once, at boot, after the bus
    /// exists). Idempotent — a second call is ignored. When set, a write that
    /// parks for approval publishes an `awaiting_approval` envelope for the
    /// alert router to fan out to an approver's channel.
    pub fn set_bus(&self, bus: Arc<dyn daimon_core::BusHandle>) {
        let _ = self.bus.set(bus);
    }

    /// Publish the `awaiting_approval` envelope for a newly-parked approval.
    /// FAIL-SOFT: the Postgres approval row (already inserted) is the source of
    /// truth; a failed/absent bus publish MUST NOT affect the approval flow —
    /// the write still parks and the console path is unaffected. No secret ever
    /// rides the envelope (approval id + capability + target only).
    async fn publish_awaiting_approval(
        &self,
        id: uuid::Uuid,
        capability: &str,
        target_ref: Option<&str>,
        actor_id: &str,
    ) {
        let Some(bus) = self.bus.get() else { return };
        let Some(env) = awaiting_approval_envelope(id, capability, target_ref, actor_id) else {
            return;
        };
        if let Err(e) = bus.send(env).await {
            tracing::debug!(error = %e, "awaiting_approval bus publish failed (best-effort)");
        }
    }

    /// Update the approval timeout live (seconds). The next `pre_flight` that
    /// waits for a decision uses the new value. A zero is ignored (a zero
    /// timeout would deny every gated request instantly — a config typo must
    /// not silently break approvals); the previous value is kept.
    pub fn set_approval_timeout_secs(&self, secs: u64) {
        if secs > 0 {
            self.approval_timeout_secs.store(secs, Ordering::Relaxed);
        }
    }

    pub fn kill(&self) -> &KillState {
        &self.kill
    }

    pub fn policy(&self) -> &PolicyEngine {
        &self.policy
    }

    pub fn approvals(&self) -> &Arc<ApprovalQueue> {
        &self.approvals
    }

    pub fn approval_timeout(&self) -> Duration {
        Duration::from_secs(self.approval_timeout_secs.load(Ordering::Relaxed))
    }

    /// Pre-flight a capability invocation. Returns Ok(()) if the broker may
    /// proceed; Err with a specific variant otherwise.
    ///
    /// `is_read_only` short-circuits to allow — read capabilities skip
    /// policy + approval. This bit MUST be derived server-side by the caller
    /// (the broker derives it from the registered `Capability::is_read()`,
    /// never from a caller/LLM-supplied flag — H6/H7). The kill-switch check
    /// stays FIRST, so a read can never bypass a KILL.
    pub async fn pre_flight(
        &self,
        actor_id: &str,
        capability: &str,
        target_ref: Option<&str>,
        params: serde_json::Value,
        is_read_only: bool,
    ) -> Result<()> {
        // 1. Kill switch FIRST — it overrides everything.
        if self.kill.engaged() {
            return Err(Error::KillEngaged {
                reason: self.kill.reason(),
            });
        }
        if is_read_only {
            return Ok(());
        }
        // 2. Policy decision.
        let verdict = self.policy.evaluate(capability);
        match verdict.decision {
            Decision::Allow => Ok(()),
            Decision::Deny => Err(Error::PolicyDenied {
                reason: format!("policy denies capability `{capability}`"),
            }),
            Decision::RequireApproval => {
                let id = self
                    .approvals
                    .enqueue(actor_id, capability, target_ref, params)
                    .await?;
                tracing::info!(
                    approval = %id,
                    capability,
                    "approval required — broker parking on inbox row"
                );
                // P7-1 (FR-GW-13): notify an approver on their channel. Publish
                // AFTER the row is inserted (the row is the source of truth) and
                // fail-soft (never blocks the park below).
                self.publish_awaiting_approval(id, capability, target_ref, actor_id)
                    .await;
                let rec = self
                    .approvals
                    .wait_for_decision(id, self.approval_timeout(), Duration::from_secs(2))
                    .await?;
                match rec.status {
                    ApprovalStatus::Approved => Ok(()),
                    ApprovalStatus::Denied => Err(Error::PolicyDenied {
                        reason: format!(
                            "operator denied approval {id} for `{capability}`"
                        ),
                    }),
                    _ => Err(Error::ApprovalTimeout(id.to_string())),
                }
            }
        }
    }
}

/// P7-1 (FR-GW-13): build the `awaiting_approval` bus envelope for a parked
/// approval. Pure + testable (no bus/DB). Returns `None` only if the fixed
/// version requirement fails to parse (never in practice). The body carries the
/// approval id + capability + target + real actor — NO secret material.
fn awaiting_approval_envelope(
    id: uuid::Uuid,
    capability: &str,
    target_ref: Option<&str>,
    actor_id: &str,
) -> Option<daimon_core::AgentEnvelope> {
    let version_req = "^1".parse().ok()?;
    let body = serde_json::json!({
        "kind": "awaiting_approval",
        "approval_id": id.to_string(),
        "capability": capability,
        "target_ref": target_ref,
        "actor": actor_id,
    });
    Some(daimon_core::AgentEnvelope::new(
        daimon_core::AgentId::new("guard"),
        daimon_core::Recipient::ByCapability {
            name: "harness.alert.approval".to_string(),
            version_req,
        },
        body,
    ))
}

#[cfg(test)]
mod emit_tests {
    use super::awaiting_approval_envelope;

    #[test]
    fn envelope_carries_the_router_contract_shape_no_secret() {
        // P7-1: the guard's producer must emit exactly the shape the P6
        // alert_router::classify_approval consumer parses — approval_id +
        // capability + target + real actor, and NO secret material.
        let id = uuid::Uuid::nil();
        let env = awaiting_approval_envelope(
            id,
            "orchestrator.k8s.deploy.restart",
            Some("target://k3s-lab"),
            "user:arif",
        )
        .expect("envelope builds");
        assert_eq!(env.body["kind"], "awaiting_approval");
        assert_eq!(env.body["approval_id"], id.to_string());
        assert_eq!(env.body["capability"], "orchestrator.k8s.deploy.restart");
        assert_eq!(env.body["target_ref"], "target://k3s-lab");
        assert_eq!(env.body["actor"], "user:arif");
        // no secret keys leaked onto the envelope
        let obj = env.body.as_object().unwrap();
        assert!(!obj.keys().any(|k| k.contains("token") || k.contains("secret") || k.contains("password")));
    }
}
