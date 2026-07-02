//! TriageAgent unit tests (P3 commit 8).
//!
//! `handle` end-to-end needs a live Postgres pool (via `OrchestratorService::
//! create_plan`), so the full spawn-over-InProcBus flow is a DB-gated
//! integration test (`tests/e2e.rs`, `#[ignore]`). These pool-free units cover
//! the LOAD-BEARING invariants directly:
//!
//! - the routing contract: the agent advertises EXACTLY ONE capability,
//!   `harness.triage.anomaly` `1.0.0`, which is what the observer's
//!   `ByCapability` envelope resolves against;
//! - the anti-loop de-dupe (`admit`): the same `(source_id, metric_name)`
//!   signature is admitted ONCE inside the TTL and re-admitted after it lapses.
//!   `admit` is the exact guard `handle` gates create_plan on, so "same
//!   signature twice → one create_plan" is proven at the guard;
//! - the pure plan-intent + Incident-record shapes.

use super::*;
use daimon_memory::{
    IngestDoc, IngestStats, MemoryHealth, PreTurnContext, RecallBudget, RetrieveQuery,
    RetrievedChunk, ScoredRecord,
};
use daimon_observer::AnomalyDetected;
use daimon_runtime::{CapabilityRegistry, InProcBus};
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

fn anomaly(source_id: &str, metric: &str) -> AnomalyDetected {
    AnomalyDetected {
        anomaly_id: Uuid::new_v4(),
        source: "prometheus".into(),
        source_id: source_id.into(),
        severity: "critical".into(),
        title: "CPU saturation > 98%".into(),
        metric_name: metric.into(),
        metric_value: 99.2,
        threshold: 98.0,
        query: "node_cpu_saturation".into(),
        target_ref: Some(format!("target://{source_id}")),
    }
}

/// A memory stub that counts captures and can be flipped to fail (to prove the
/// capture path is fail-soft). Only `capture`/`health` are exercised; the rest
/// return trivial degraded/empty values.
struct StubMemory {
    captures: AtomicUsize,
    fail: bool,
}

impl StubMemory {
    fn new(fail: bool) -> Self {
        Self { captures: AtomicUsize::new(0), fail }
    }
}

#[async_trait]
impl MemoryService for StubMemory {
    async fn ingest(&self, doc: IngestDoc) -> daimon_memory::Result<IngestStats> {
        Ok(IngestStats { source_id: doc.source_id, chunks: 1, collection: "test".into() })
    }
    async fn delete(&self, _uri: &str) -> daimon_memory::Result<()> {
        Ok(())
    }
    async fn retrieve(&self, _q: &RetrieveQuery) -> daimon_memory::Result<Vec<RetrievedChunk>> {
        Ok(vec![])
    }
    async fn capture(&self, _rec: TypedRecord) -> daimon_memory::Result<String> {
        self.captures.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err(daimon_memory::Error::Unreachable("stub forced failure".into()))
        } else {
            Ok("daimon://incident/stub".into())
        }
    }
    async fn recall(&self, _q: &str, _b: RecallBudget) -> daimon_memory::Result<Vec<ScoredRecord>> {
        Ok(vec![])
    }
    async fn pre_turn_recall(&self, _m: &str, _b: RecallBudget) -> PreTurnContext {
        PreTurnContext::degraded()
    }
    async fn health(&self) -> MemoryHealth {
        MemoryHealth { reachable: true, detail: None }
    }
}

/// Build a TriageAgent with a real (dispatcher over an in-proc bus) + a stub
/// memory, but WITHOUT a live orchestrator pool. Used only for the guard-level
/// tests that never reach `create_plan`.
fn agent_with_stub_memory(fail_capture: bool) -> (TriageAgent, Arc<StubMemory>) {
    // A pool-free Dispatcher (empty registry) — held but not exercised in P3.
    let bus = InProcBus::new();
    let registry = CapabilityRegistry::new();
    let dispatcher = Dispatcher::new(bus, registry);

    // We cannot build a real OrchestratorService without a Pool, and the
    // guard-level tests never call it. Construct the agent fields directly.
    let memory = Arc::new(StubMemory::new(fail_capture));
    let agent = TriageAgent {
        id: AgentId::new("agent:triage"),
        caps: vec![Capability::read_only(TRIAGE_CAPABILITY, Version::new(1, 0, 0))],
        // SAFETY: guard-level tests never touch the orchestrator. We install a
        // dangling Arc via a never-called path by using a zero-field service is
        // impossible (needs a Pool), so these tests exercise only `admit`,
        // `capture_incident`, and the pure helpers — none of which read
        // `orchestrator`. See the DB-gated e2e test for the create_plan path.
        orchestrator: orchestrator_placeholder(),
        dispatcher,
        memory: memory.clone(),
        seen: Mutex::new(HashMap::new()),
        dedupe_ttl: Duration::from_millis(200),
    };
    (agent, memory)
}

/// The orchestrator field is required to construct the struct but is never read
/// by the guard-level tests. Building a real `OrchestratorService` needs a
/// Postgres `Pool`; since these tests must run pool-free, we obtain one lazily
/// only in the DB-gated e2e path. Here we panic if it is ever *used*, which the
/// guard tests never do — they exercise `admit`/`capture_incident` only.
fn orchestrator_placeholder() -> Arc<OrchestratorService> {
    // `build_pool` parses the DSN but does NOT connect — deadpool connects
    // lazily on the first `.get()`. The guard tests never call create_plan (so
    // never `.get()`), so an unconnected pool against an unroutable DSN is a
    // safe placeholder. The create_plan path is proven in the DB-gated e2e test.
    let pool = daimon_db::build_pool("postgres://triage-test-unused@127.0.0.1:1/none")
        .expect("build_pool parses DSN without connecting");
    let bus = InProcBus::new();
    let registry = CapabilityRegistry::new();
    let dispatcher = Dispatcher::new(bus, registry);
    Arc::new(OrchestratorService::new(pool, dispatcher))
}

#[test]
fn advertises_exactly_the_triage_capability() {
    let (agent, _mem) = agent_with_stub_memory(false);
    let caps = agent.capabilities();
    assert_eq!(caps.len(), 1, "triage advertises EXACTLY one capability");
    assert_eq!(caps[0].name, "harness.triage.anomaly");
    assert_eq!(caps[0].version, Version::new(1, 0, 0));
    // Loop safety: the capability declares NO compensator and is NOT
    // irreversible — it is a pure inbound trigger, never dispatched as a write.
    // (`is_read()` is intentionally false: the name carries no read verb, but
    // that guard only governs broker write-gating, which triage never invokes
    // for this capability — routing matches on name+version only.)
    assert!(caps[0].compensating.is_none());
    assert!(!caps[0].irreversible);
    // And its id is the supervised triage identity.
    assert_eq!(agent.id().as_str(), "agent:triage");
}

#[tokio::test]
async fn admit_dedupes_same_signature_within_ttl() {
    let (agent, _mem) = agent_with_stub_memory(false);
    // First sighting of (node-01, cpu) is admitted.
    assert!(agent.admit("node-01", "node.cpu.saturation_pct").await);
    // Immediate repeat of the SAME signature is swallowed (anti-loop).
    assert!(!agent.admit("node-01", "node.cpu.saturation_pct").await);
    // A DIFFERENT source is independent — admitted.
    assert!(agent.admit("node-02", "node.cpu.saturation_pct").await);
    // A DIFFERENT metric on the same source is independent — admitted.
    assert!(agent.admit("node-01", "node.memory.pressure_pct").await);
}

#[tokio::test]
async fn admit_readmits_after_ttl_lapses() {
    let (agent, _mem) = agent_with_stub_memory(false); // ttl = 200ms
    assert!(agent.admit("node-01", "cpu").await);
    assert!(!agent.admit("node-01", "cpu").await);
    tokio::time::sleep(Duration::from_millis(250)).await;
    // TTL lapsed — the same breach is admitted again (a persistent problem
    // re-opens triage, it isn't silenced forever).
    assert!(agent.admit("node-01", "cpu").await);
}

#[tokio::test]
async fn capture_incident_is_fail_soft() {
    // A memory backend that ERRORS on capture must NOT panic or propagate —
    // capture_incident swallows it (memory is the aid, plan+audit are truth).
    let (agent, mem) = agent_with_stub_memory(true);
    let a = anomaly("node-01", "node.cpu.saturation_pct");
    // Should complete without panicking despite the forced capture error.
    agent.capture_incident(&a, Uuid::new_v4()).await;
    assert_eq!(mem.captures.load(Ordering::SeqCst), 1, "capture was attempted");
}

#[test]
fn triage_intent_and_incident_record_shapes() {
    let a = anomaly("node-01", "node.cpu.saturation_pct");
    let intent = triage_intent(&a);
    assert_eq!(
        intent,
        "triage: CPU saturation > 98% (node.cpu.saturation_pct = 99.2 > 98)"
    );

    let plan_id = Uuid::new_v4();
    let rec = incident_record(&a, plan_id);
    match rec.body {
        TypedBody::Incident { title, impact, resolution } => {
            assert_eq!(title, "anomaly: CPU saturation > 98%");
            assert!(impact.contains("node-01"));
            assert!(impact.contains("node.cpu.saturation_pct"));
            assert!(impact.contains("98")); // threshold present
            assert!(resolution.contains(&plan_id.to_string()));
            assert!(resolution.contains("pending operator"));
        }
        other => panic!("expected Incident, got {other:?}"),
    }
    assert!(rec.namespace.is_none());
}
