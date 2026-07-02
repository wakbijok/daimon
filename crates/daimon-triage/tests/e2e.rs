//! DB-gated end-to-end: a real TriageAgent spawned under a Supervisor consumes
//! an observer `AnomalyDetected` envelope over the in-proc bus, opens a
//! PERSISTED-but-NOT-RUN plan, and de-dupes a repeat signature (P3 commit 8,
//! AC-P3-01 + anti-loop).
//!
//! Gated `#[ignore]` (needs Postgres). Run with:
//! ```
//! just pg-up && just pg-migrate
//! cargo test -p daimon-triage --test e2e -- --ignored --nocapture
//! ```
//! Env: `DAIMON_PG_URL` (defaults to `postgres://wakbijak@localhost:5432/daimon`).

use std::sync::Arc;
use std::time::Duration;

use daimon_core::{Agent, AgentEnvelope, AgentId, Recipient};
use daimon_db::Pool;
use daimon_memory::NullMemory;
use daimon_observer::AnomalyDetected;
use daimon_orchestrator::{OrchestratorService, PlanStatus};
use daimon_runtime::{CapabilityRegistry, Dispatcher, InProcBus, Supervisor};
use daimon_triage::TriageAgent;
use uuid::Uuid;

fn pg_url() -> String {
    std::env::var("DAIMON_PG_URL")
        .unwrap_or_else(|_| "postgres://wakbijak@localhost:5432/daimon".to_string())
}

async fn pool() -> Pool {
    let mgr = deadpool_postgres::Manager::new(
        pg_url().parse().expect("pg url"),
        tokio_postgres::NoTls,
    );
    deadpool_postgres::Pool::builder(mgr)
        .max_size(4)
        .build()
        .expect("pool")
}

fn anomaly(source_id: &str) -> AnomalyDetected {
    AnomalyDetected {
        anomaly_id: Uuid::new_v4(),
        source: "prometheus".into(),
        source_id: source_id.into(),
        severity: "critical".into(),
        // Unique title so we can find exactly the plans this test opened.
        title: format!("e2e triage {}", Uuid::new_v4()),
        metric_name: "node.cpu.saturation_pct".into(),
        metric_value: 99.5,
        threshold: 98.0,
        query: "node_cpu_saturation".into(),
        target_ref: Some(format!("target://{source_id}")),
    }
}

fn envelope(a: &AnomalyDetected) -> AgentEnvelope {
    AgentEnvelope::new(
        AgentId::new("observer"),
        Recipient::ByCapability {
            name: "harness.triage.anomaly".into(),
            version_req: "^1".parse().unwrap(),
        },
        serde_json::to_value(a).unwrap(),
    )
}

#[tokio::test]
#[ignore]
async fn anomaly_opens_one_persisted_not_run_plan_and_dedupes_repeat() {
    let pool = pool().await;

    // The shared bus + registry + supervisor, exactly like main.rs.
    let bus = InProcBus::new();
    let registry = CapabilityRegistry::new();
    let supervisor = Arc::new(Supervisor::new(bus.clone(), registry.clone()));

    let orchestrator = Arc::new(OrchestratorService::new(
        pool.clone(),
        Dispatcher::new(bus.clone(), registry.clone()),
    ));

    let triage = TriageAgent::new(
        orchestrator.clone(),
        Dispatcher::new(bus.clone(), registry.clone()),
        Arc::new(NullMemory),
    );
    supervisor
        .spawn(Arc::new(triage) as Arc<dyn Agent>)
        .await
        .expect("spawn triage");
    // Let the supervised runner subscribe before we publish.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Snapshot the plan count before.
    let before = orchestrator.list_plans(1000).await.expect("list before").len();

    // FIRST anomaly — routes ByCapability to triage → one plan opened.
    let a = anomaly("node-e2e-01");
    let expected_intent = daimon_triage::triage_intent(&a);
    bus.handle().send(envelope(&a)).await.expect("send 1");
    tokio::time::sleep(Duration::from_millis(400)).await;

    // SECOND anomaly — SAME (source_id, metric_name) signature → swallowed.
    let mut a2 = anomaly("node-e2e-01");
    a2.metric_name = a.metric_name.clone(); // ensure same signature
    bus.handle().send(envelope(&a2)).await.expect("send 2");
    tokio::time::sleep(Duration::from_millis(400)).await;

    let plans = orchestrator.list_plans(1000).await.expect("list after");
    let after = plans.len();

    // Exactly ONE new plan (anti-loop: the repeat did not open a second).
    assert_eq!(after, before + 1, "exactly one plan opened for the signature");

    // Find the plan we opened and assert it is PERSISTED but NOT RUN.
    let ours = plans
        .iter()
        .find(|p| p.intent == expected_intent)
        .expect("our triage plan exists");
    assert_eq!(
        ours.status,
        PlanStatus::Planning,
        "triage plan must be created-not-run (Planning), never executing/succeeded"
    );
    assert!(ours.started_at.is_none(), "plan was never started (not run)");

    // It is a context-only plan (no steps in P3).
    let steps = orchestrator.list_steps(ours.id).await.expect("list steps");
    assert!(steps.is_empty(), "P3 triage opens a context-only plan (no steps)");
}
