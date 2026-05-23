//! Live smoke test against a running NornicDB instance.
//!
//! Run with: `just nornicdb-up && cargo test -p daimon-graph --test nornicdb_smoke -- --ignored --nocapture`
//!
//! Set `DAIMON_GRAPH_URL` to override the default `bolt://localhost:7687`.

use chrono::Utc;
use daimon_graph::{
    ensure_schema, GraphClient, GraphPlan, GraphPlanStep, NornicGraphClient, TargetRef,
};
use uuid::Uuid;

fn url() -> String {
    std::env::var("DAIMON_GRAPH_URL").unwrap_or_else(|_| "bolt://localhost:7687".to_string())
}

async fn connect() -> NornicGraphClient {
    NornicGraphClient::connect(&url(), "", "")
        .await
        .expect("connect to NornicDB; is it running? `just nornicdb-up`")
}

#[tokio::test]
#[ignore]
async fn schema_bootstrap_idempotent() {
    let g = connect().await;
    ensure_schema(&g).await.expect("first bootstrap");
    ensure_schema(&g).await.expect("second bootstrap should be a no-op");
}

#[tokio::test]
#[ignore]
async fn persist_plan_and_query_blast_radius() {
    let g = connect().await;
    ensure_schema(&g).await.expect("schema");

    let tenant = Uuid::new_v4();
    let plan_id = Uuid::new_v4();
    let step1 = Uuid::new_v4();
    let step2 = Uuid::new_v4();

    let plan = GraphPlan {
        id: plan_id,
        tenant_id: tenant,
        intent: "smoke test plan".into(),
        created_at: Utc::now(),
        steps: vec![
            GraphPlanStep {
                id: step1,
                capability_name: "mt.list_interfaces".into(),
                capability_version: "1.0.0".into(),
                target_ref: TargetRef("target://mikrotik-edge".into()),
                depends_on: vec![],
            },
            GraphPlanStep {
                id: step2,
                capability_name: "mt.run_command_allowlisted".into(),
                capability_version: "1.0.0".into(),
                target_ref: TargetRef("target://mikrotik-edge".into()),
                depends_on: vec![step1],
            },
        ],
    };

    g.persist_plan(&plan).await.expect("persist_plan");

    // Blast radius from the target should now include the Plan + PlanSteps +
    // Capabilities reachable from it.
    let entries = g
        .blast_radius(tenant, &TargetRef("target://mikrotik-edge".into()), 4)
        .await
        .expect("blast_radius");
    assert!(
        !entries.is_empty(),
        "blast_radius should return reachable dependent nodes, got empty"
    );
    let labels: Vec<_> = entries.iter().map(|e| e.label.as_str()).collect();
    assert!(
        labels.iter().any(|l| l.contains("mt.list_interfaces")),
        "expected mt.list_interfaces capability in blast radius, got {labels:?}"
    );
}

#[tokio::test]
#[ignore]
async fn upsert_target_and_dependency() {
    let g = connect().await;
    ensure_schema(&g).await.expect("schema");

    let tenant = Uuid::new_v4();
    let a = TargetRef("target://test-a".into());
    let b = TargetRef("target://test-b".into());

    g.upsert_target(tenant, &a, serde_json::json!({"kind": "vlan", "id": 20}))
        .await
        .expect("upsert_target a");
    g.upsert_target(tenant, &b, serde_json::json!({"kind": "workload", "name": "tikTok"}))
        .await
        .expect("upsert_target b");
    g.declare_dependency(tenant, &b, &a)
        .await
        .expect("declare_dependency b -> a");

    // From `a`, blast radius should include `b`.
    let entries = g.blast_radius(tenant, &a, 2).await.expect("blast_radius");
    assert!(
        entries.iter().any(|e| e.label.contains("target://test-b")),
        "expected target://test-b in blast radius of test-a, got {entries:?}"
    );
}
