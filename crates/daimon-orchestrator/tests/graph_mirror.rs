//! Phase 8 wiring proof — Orchestrator persists a plan and mirrors it
//! into NornicDB; blast-radius from the target returns the plan + step +
//! capability nodes.
//!
//! Gated `#[ignore]`. Run with:
//! ```
//! just pg-up && just pg-migrate && just nornicdb-up
//! cargo test -p daimon-orchestrator --test graph_mirror -- --ignored --nocapture
//! ```
//!
//! Env overrides:
//! - `DAIMON_PG_URL` (defaults to `postgres://wakbijak@localhost:5432/daimon`)
//! - `DAIMON_GRAPH_URL` (defaults to `bolt://localhost:7687`)

use std::sync::Arc;

use daimon_db::Pool;
use daimon_graph::{
    ensure_schema, BlastRadiusEntry, GraphClient, NornicGraphClient, TargetRef as GraphTargetRef,
};
use daimon_orchestrator::{OrchestratorService, StepDef};
use daimon_runtime::{CapabilityRegistry, Dispatcher, InProcBus};
use serde_json::json;
use uuid::Uuid;

fn pg_url() -> String {
    std::env::var("DAIMON_PG_URL")
        .unwrap_or_else(|_| "postgres://wakbijak@localhost:5432/daimon".to_string())
}

fn graph_url() -> String {
    std::env::var("DAIMON_GRAPH_URL").unwrap_or_else(|_| "bolt://localhost:7687".to_string())
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

/// This test only exercises `create_plan` (persistence + graph mirror), never
/// `run_plan`/`dispatch_step`, so a bare `Dispatcher` over an empty in-proc bus
/// suffices — no driver is spawned and no capability is dispatched.
fn stub_dispatcher() -> Dispatcher {
    Dispatcher::new(InProcBus::new(), CapabilityRegistry::new())
}

async fn graph_client() -> Arc<dyn GraphClient> {
    let client = NornicGraphClient::connect(&graph_url(), "", "")
        .await
        .expect("connect NornicDB; is it running? `just nornicdb-up`");
    ensure_schema(&client).await.expect("schema bootstrap");
    Arc::new(client)
}

#[tokio::test]
#[ignore]
async fn orchestrator_mirrors_plan_to_graph_and_blast_radius_finds_capability() {
    // Setup the same way main.rs does.
    let pool = pool().await;
    let dispatcher = stub_dispatcher();
    let graph = graph_client().await;

    let service = OrchestratorService::new(pool.clone(), dispatcher).with_graph(graph.clone());

    // A 2-step plan referencing a unique target so we don't collide with
    // prior test runs.
    let target_ref = format!("target://orch-graph-{}", Uuid::new_v4());
    let steps = vec![
        StepDef {
            capability_name: "mt.list_interfaces".into(),
            capability_version: "1.0.0".into(),
            target_ref: Some(target_ref.clone()),
            credential_ref: None,
            params: json!({"command": "/interface print", "is_read_only": true}),
            depends_on_index: vec![],
        },
        StepDef {
            capability_name: "mt.run_command_allowlisted".into(),
            capability_version: "1.0.0".into(),
            target_ref: Some(target_ref.clone()),
            credential_ref: None,
            params: json!({
                "command": "/system identity print",
                "is_read_only": true,
            }),
            depends_on_index: vec![0],
        },
    ];

    let plan = service
        .create_plan(None, "orchestrator graph mirror smoke test", steps)
        .await
        .expect("create_plan");

    // Blast radius from the target should now include the Capability node(s)
    // by name and the Plan id.
    let entries: Vec<BlastRadiusEntry> = graph
        .blast_radius(&GraphTargetRef::from(target_ref.as_str()), 4)
        .await
        .expect("blast_radius");

    assert!(
        !entries.is_empty(),
        "expected at least one node reachable from {target_ref}"
    );
    let labels: Vec<_> = entries.iter().map(|e| e.label.as_str()).collect();
    assert!(
        labels.iter().any(|l| l.contains("mt.list_interfaces")),
        "expected mt.list_interfaces in blast radius, got {labels:?}"
    );
    assert!(
        labels.iter().any(|l| *l == plan.id.to_string()),
        "expected plan id {} in blast radius, got {labels:?}",
        plan.id
    );
}
