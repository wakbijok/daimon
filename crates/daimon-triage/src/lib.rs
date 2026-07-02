//! The **TriageAgent** — the detect→triage hop of daimon's AIOps loop
//! (P3 commit 8, FR-MON-09, AC-P3-01).
//!
//! # Routing contract (load-bearing)
//!
//! The observer emits an [`AnomalyDetected`] envelope addressed
//! `Recipient::ByCapability { name: "harness.triage.anomaly", version_req: "^1" }`
//! (see `daimon-observer`). A `ByCapability` envelope reaches an agent ONLY if
//! that agent is registered under the Supervisor advertising a matching
//! capability. So the TriageAgent advertises **exactly one** capability —
//! [`Capability::read_only("harness.triage.anomaly", 1.0.0)`] — and MUST be
//! spawned under the same Supervisor as the drivers. That single capability is
//! what makes the observer's envelope land here.
//!
//! # Loop safety (FR-MON-09)
//!
//! Triage is the point where the AIOps loop could close on itself
//! (anomaly → plan → … → anomaly). Two invariants keep it open:
//!
//! 1. **Signature de-dupe.** An in-memory window keyed on `(source_id,
//!    metric_name)` swallows a repeat within [`Self::dedupe_ttl`]. The same
//!    breach firing every scrape does NOT open a new plan each time.
//! 2. **Create, do not run.** Triage PERSISTS a plan but NEVER calls
//!    `run_plan`. Remediation stays behind `run_plan`'s default-deny +
//!    approval gate. The loop cannot close autonomously because nothing here
//!    executes a write.
//!
//! # What the plan contains (P3 scope)
//!
//! P3 opens a **context-only** plan (empty steps) — see [`TriageAgent::handle`]
//! for the rationale. Full diagnose→read→remediate intelligence is deferred
//! (the `dispatcher` is held for that P4 hop). Every memory write is fail-soft:
//! a capture failure logs and is swallowed, never panics, never blocks.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use daimon_core::{
    Agent, AgentContext, AgentEnvelope, AgentId, Capability, CoreError,
};
use daimon_memory::{MemoryService, TypedBody, TypedRecord};
use daimon_observer::AnomalyDetected;
use daimon_orchestrator::OrchestratorService;
use daimon_runtime::Dispatcher;
use semver::Version;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Default de-dupe window: a `(source_id, metric_name)` signature seen within
/// this span is treated as already-in-flight (FR-MON-09 anti-loop).
const DEDUPE_TTL: Duration = Duration::from_secs(600); // 10 minutes

/// The single capability the triage agent advertises. The observer addresses
/// its `AnomalyDetected` envelope `ByCapability` this name; advertising it here
/// is what routes the envelope to this agent.
pub const TRIAGE_CAPABILITY: &str = "harness.triage.anomaly";

/// The supervised coordinator that turns an observer anomaly into a persisted
/// (never auto-run) triage plan + an Incident record.
pub struct TriageAgent {
    id: AgentId,
    caps: Vec<Capability>,
    orchestrator: Arc<OrchestratorService>,
    /// Held for the P4 diagnose hop (read_state over the bus → Findings →
    /// StepDefs). P3 opens a context-only plan and does NOT dispatch, so this
    /// is intentionally not yet exercised.
    #[allow(dead_code)]
    dispatcher: Dispatcher,
    memory: Arc<dyn MemoryService>,
    /// In-memory anti-loop de-dupe: last-seen instant per `(source_id,
    /// metric_name)` signature. Entries older than `dedupe_ttl` are ignored
    /// (and lazily overwritten), so this map never needs a sweeper.
    seen: Mutex<HashMap<(String, String), Instant>>,
    dedupe_ttl: Duration,
}

impl TriageAgent {
    /// Build a triage agent. `id` is conventionally `"agent:triage"`.
    pub fn new(
        orchestrator: Arc<OrchestratorService>,
        dispatcher: Dispatcher,
        memory: Arc<dyn MemoryService>,
    ) -> Self {
        Self {
            id: AgentId::new("agent:triage"),
            // EXACTLY ONE capability — the observer's ByCapability address.
            caps: vec![Capability::read_only(TRIAGE_CAPABILITY, Version::new(1, 0, 0))],
            orchestrator,
            dispatcher,
            memory,
            seen: Mutex::new(HashMap::new()),
            dedupe_ttl: DEDUPE_TTL,
        }
    }

    /// Override the de-dupe window (tests use a short one).
    pub fn with_dedupe_ttl(mut self, ttl: Duration) -> Self {
        self.dedupe_ttl = ttl;
        self
    }

    /// The de-dupe check. Returns `true` if this signature is a fresh anomaly
    /// (records it and lets triage proceed); `false` if it was already seen
    /// within the TTL (triage skips). Load-bearing anti-loop guard.
    async fn admit(&self, source_id: &str, metric_name: &str) -> bool {
        let key = (source_id.to_string(), metric_name.to_string());
        let now = Instant::now();
        let mut seen = self.seen.lock().await;
        if let Some(prev) = seen.get(&key) {
            if now.duration_since(*prev) < self.dedupe_ttl {
                return false; // already in-flight — swallow
            }
        }
        seen.insert(key, now);
        true
    }

    /// Capture an Incident for a triaged anomaly. Fail-soft: any error is
    /// logged and swallowed — memory is the aid, the plan + audit are the
    /// truth, so a memory fault must never fail triage.
    async fn capture_incident(&self, anomaly: &AnomalyDetected, plan_id: uuid::Uuid) {
        let record = incident_record(anomaly, plan_id);
        match self.memory.capture(record).await {
            Ok(uri) => info!(uri = %uri, anomaly_id = %anomaly.anomaly_id, "triage incident captured"),
            Err(e) => warn!(
                error = %e,
                anomaly_id = %anomaly.anomaly_id,
                "triage incident capture failed (fail-soft — ignored)"
            ),
        }
    }
}

#[async_trait]
impl Agent for TriageAgent {
    fn id(&self) -> &AgentId {
        &self.id
    }

    fn capabilities(&self) -> &[Capability] {
        &self.caps
    }

    async fn handle(&self, env: AgentEnvelope, _ctx: AgentContext) -> Result<(), CoreError> {
        // Decode the anomaly. A malformed body is the one genuine error we
        // surface (it points at an observer/schema drift). #[from] serde maps it.
        let anomaly: AnomalyDetected = serde_json::from_value(env.body)?;

        // (1) LOOP GUARD — anti-loop de-dupe on (source_id, metric_name).
        if !self.admit(&anomaly.source_id, &anomaly.metric_name).await {
            info!(
                anomaly_id = %anomaly.anomaly_id,
                source_id = %anomaly.source_id,
                metric = %anomaly.metric_name,
                "triage skipped — signature already in-flight (anti-loop)"
            );
            return Ok(());
        }

        // (2) Open a triage plan. P3 SCOPE: this is a CONTEXT-ONLY plan (empty
        // steps). Rationale — an `AnomalyDetected` carries a best-effort
        // `target_ref` + a metric name, but NO capability name, and no read
        // capability is guaranteed registered for an arbitrary target. Emitting
        // a step that names an unresolvable capability would produce an
        // unrunnable plan; a step that names a wrong one is worse than none. So
        // triage records the intent and leaves step authoring to the operator
        // (plan_from_intent) or the deferred P4 diagnose hop. The plan is
        // PERSISTED but NOT run — remediation stays behind run_plan's
        // guard+approval; triage NEVER calls run_plan.
        let intent = triage_intent(&anomaly);
        let plan = match self.orchestrator.create_plan(None, &intent, Vec::new()).await {
            Ok(plan) => plan,
            Err(e) => {
                // A plan-create failure (e.g. transient DB fault) is logged;
                // returning Ok keeps the agent alive (no supervisor restart) and
                // lets the next anomaly re-trigger. The dedupe entry we just set
                // will expire, so a genuine breach isn't permanently swallowed.
                warn!(
                    error = %e,
                    anomaly_id = %anomaly.anomaly_id,
                    "triage create_plan failed — no plan opened this cycle"
                );
                return Ok(());
            }
        };
        info!(
            plan_id = %plan.id,
            anomaly_id = %anomaly.anomaly_id,
            "triage plan created (context-only, PENDING operator — not auto-run)"
        );

        // (3) Capture the Incident — fail-soft.
        self.capture_incident(&anomaly, plan.id).await;

        Ok(())
    }
}

/// The triage plan intent line for an anomaly. Pure (no I/O) so it is
/// unit-testable and stable across the create_plan call.
pub fn triage_intent(anomaly: &AnomalyDetected) -> String {
    format!(
        "triage: {} ({} = {} > {})",
        anomaly.title, anomaly.metric_name, anomaly.metric_value, anomaly.threshold,
    )
}

/// The Incident record triage captures for an anomaly. Pure so the record shape
/// (impact + resolution) is unit-testable without a live memory backend.
pub fn incident_record(anomaly: &AnomalyDetected, plan_id: uuid::Uuid) -> TypedRecord {
    let impact = format!(
        "{} on {} — {} = {} (breached threshold {})",
        anomaly.title,
        anomaly.source_id,
        anomaly.metric_name,
        anomaly.metric_value,
        anomaly.threshold,
    );
    TypedRecord {
        body: TypedBody::Incident {
            title: format!("anomaly: {}", anomaly.title),
            impact,
            resolution: format!("triage plan {plan_id} created (pending operator)"),
        },
        namespace: None,
    }
}

#[cfg(test)]
mod tests;
