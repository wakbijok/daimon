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
}

impl Guard {
    pub fn new(kill: KillState, policy: PolicyEngine, approvals: ApprovalQueue) -> Self {
        Self {
            kill,
            policy: Arc::new(policy),
            approvals: Arc::new(approvals),
            approval_timeout_secs: Arc::new(AtomicU64::new(DEFAULT_APPROVAL_TIMEOUT_SECS)),
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
