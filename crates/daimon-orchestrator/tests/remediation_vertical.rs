//! End-to-end remediation vertical — the AIOps loop driven in-process against
//! an ephemeral Postgres + StubTransport.
//!
//! This is the capstone proof: detect→triage→APPROVE→remediate→ROLLBACK→remember.
//! The analog of the original "locked TikTok-block vertical" e2e test, extended
//! with saga rollback + memory capture. It assembles the REAL wiring — no
//! shortcuts around the guard, the approval gate, or the saga:
//!
//! - InProcBus + CapabilityRegistry + Supervisor
//! - Broker::new(InMemoryRegistry, StubVaultClient, {ssh: StubTransport})
//!     .with_guard(Guard{ policy: network.routeros.* = require_approval, KillSwitch })
//! - a real RouterOsDriver(agent) spawned under the supervisor
//! - OrchestratorService::new(pool, dispatcher).with_memory(recording stub)
//!
//! Flow exercised (each hop asserted):
//!   1. run_plan → step0 (firewall_add_drop_rule, a WRITE) is approval-gated
//!   2. a background "operator" task approves the pending row → StubTransport
//!      executes the add → step0 Succeeded
//!   3. step1 (a bogus capability) fails closed at dispatch → plan fail-stops
//!   4. SAGA rollback dispatches step0's compensator firewall_remove_rule (also
//!      approval-gated → operator approves → StubTransport executes the remove)
//!      → step0 marked Compensated
//!   5. finish_plan captures a Failed→Incident record into the recording memory
//!
//! Gated `#[ignore]` (needs a live PG) like graph_mirror.rs. Run with:
//! ```
//! LC_ALL=C LANG=C cargo test -p daimon-orchestrator \
//!   --test remediation_vertical -- --ignored --nocapture
//! ```
//! Env override: `DAIMON_PG_URL` (an ephemeral PG the harness provides).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

use daimon_audit::{ActionKind, AuditFilter, AuditSink, PostgresAuditSink};
use daimon_broker::{Broker, TransportKind};
use daimon_core::{Agent, AgentId};
use daimon_db::Pool;
use daimon_driver_firewall_routeros::RouterOsDriver;
use daimon_guard::{
    ApprovalQueue, ApprovalStatus, Decision, Guard, KillState, PolicyEngine,
};
use daimon_inventory::{
    InMemoryRegistry, Inventory, ManagedTarget, TargetKind, TargetRef,
    TransportKind as InvTransportKind,
};
use daimon_memory::{
    IngestDoc, IngestStats, MemoryHealth, MemoryService, PreTurnContext, RecallBudget,
    RecordKind, RetrieveQuery, RetrievedChunk, ScoredRecord, TypedRecord,
};
use daimon_orchestrator::{OrchestratorService, PlanStatus, StepDef, StepStatus};
use daimon_runtime::{CapabilityRegistry, Dispatcher, InProcBus, Supervisor};
use daimon_transport::{StubTransport, Transport};
use daimon_vault::{Credential, MasterKey, PostgresVaultClient};

fn pg_url() -> String {
    std::env::var("DAIMON_PG_URL")
        .unwrap_or_else(|_| "postgres://wakbijak@localhost:5432/daimon".to_string())
}

fn pool() -> Pool {
    daimon_db::build_pool(&pg_url()).expect("build pool")
}

// ---------------------------------------------------------------------------
// A recording MemoryService — captures every TypedRecord so we can assert the
// Incident capture from the failed plan. Everything else is a no-op stub.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct RecordingMemory {
    captured: StdMutex<Vec<TypedRecord>>,
}

impl RecordingMemory {
    fn captured(&self) -> Vec<TypedRecord> {
        self.captured.lock().unwrap().clone()
    }
}

#[async_trait]
impl MemoryService for RecordingMemory {
    async fn ingest(&self, doc: IngestDoc) -> daimon_memory::Result<IngestStats> {
        Ok(IngestStats { source_id: doc.source_id, chunks: 1, collection: "test".into() })
    }
    async fn delete(&self, _uri: &str) -> daimon_memory::Result<()> {
        Ok(())
    }
    async fn retrieve(&self, _q: &RetrieveQuery) -> daimon_memory::Result<Vec<RetrievedChunk>> {
        Ok(vec![])
    }
    async fn capture(&self, rec: TypedRecord) -> daimon_memory::Result<String> {
        let uri = format!("daimon://plan/{}", rec.body.kind().as_str());
        self.captured.lock().unwrap().push(rec);
        Ok(uri)
    }
    async fn recall(&self, _q: &str, _b: RecallBudget) -> daimon_memory::Result<Vec<ScoredRecord>> {
        Ok(vec![])
    }
    async fn pre_turn_recall(&self, _m: &str, _b: RecallBudget) -> PreTurnContext {
        PreTurnContext::degraded()
    }
    async fn health(&self) -> MemoryHealth {
        MemoryHealth::default()
    }
}

// ---------------------------------------------------------------------------
// The background "operator" — polls the approvals table and approves every
// pending row via the REAL ApprovalQueue::decide mechanism (the exact UPDATE
// the /admin/approvals UI performs: status→approved, decided_at=now(),
// decided_by=<operator user>). This unblocks Guard::wait_for_decision.
//
// `decided_by` has a FK to public.users(id), so we seed one operator user and
// use its id — exercising the true decision-record path, not a NULL shortcut.
// ---------------------------------------------------------------------------

async fn seed_operator(pool: &Pool) -> Uuid {
    let client = pool.get().await.expect("pool");
    let row = client
        .query_one(
            "INSERT INTO public.users (username, password_hash, status)
             VALUES ($1, $2, 'active')
             RETURNING id",
            &[
                &format!("e2e-operator-{}", Uuid::new_v4()),
                &"x-not-a-real-hash",
            ],
        )
        .await
        .expect("seed operator user");
    row.get(0)
}

// ---------------------------------------------------------------------------
// The test.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn remediation_vertical_detect_approve_remediate_rollback_remember() {
    // (1) Postgres + migrations. plans/plan_steps/approvals/audit tables exist.
    daimon_db::run_migrations(&pg_url()).await.expect("run migrations");
    let pool = pool();
    let operator_user = seed_operator(&pool).await;

    // Per-run unique names so the test is idempotent across re-runs against the
    // same DB (the vault credential `name` + inventory ref are org-wide unique).
    let run = Uuid::new_v4().simple().to_string();
    let cred_name = format!("mikrotik-edge-{run}");
    let target = format!("target://mikrotik-edge-{run}");
    let vault_ref = format!("vault://infra/network/{cred_name}");
    println!("[setup] migrations applied; operator user = {operator_user}; run = {run}");

    // (2) Assemble the REAL stack ------------------------------------------------
    let bus = InProcBus::new();
    let registry = CapabilityRegistry::new();
    let supervisor = Arc::new(Supervisor::new(bus.clone(), registry.clone()));

    // Inventory + vault + transport (broker-only crates, `for-broker` on).
    // We use the REAL PostgresVaultClient (not the stub) so we can drive the
    // production broker (`with_production_admin`) — that constructor is the only
    // one that carries an audit sink, and audit.events is the source of truth for
    // "the guarded write executed". A dev master key seals the credential row.
    let inventory = Arc::new(InMemoryRegistry::new());
    let master_key = MasterKey::from_bytes([42u8; 32]);
    let vault = Arc::new(PostgresVaultClient::new(pool.clone(), master_key));
    let ssh_transport = Arc::new(StubTransport::new("ssh"));

    // Register the managed target so the broker resolves target://mikrotik-edge
    // → StubTransport, and seed the credential the broker resolves for it. The
    // Path ref `vault://infra/network/mikrotik-edge` resolves by its trailing
    // `item` segment (`mikrotik-edge`) → the vault row name.
    inventory
        .upsert(ManagedTarget {
            r#ref: TargetRef::parse(&target).unwrap(),
            kind: TargetKind::Network,
            transport: InvTransportKind::Ssh,
            host: "10.100.10.1".into(),
            port: 22,
            credential_ref: vault_ref.clone(),
            labels: Default::default(),
            capabilities: vec![],
        })
        .await
        .expect("register target");
    vault
        .create(
            &cred_name,
            Credential::SshKey {
                username: "arif".into(),
                private_key_pem: "-----BEGIN OPENSSH PRIVATE KEY-----\nstub\n-----END OPENSSH PRIVATE KEY-----".into(),
                passphrase: None,
            },
        )
        .await
        .expect("seed vault credential");

    let mut transports: HashMap<TransportKind, Arc<dyn Transport>> = HashMap::new();
    transports.insert(TransportKind::Ssh, ssh_transport.clone());

    // Guard: PolicyEngine with network.routeros.* = require_approval + a
    // (disengaged) KillSwitch. This is the REAL gate — writes MUST get an
    // operator approval to proceed.
    let policy = PolicyEngine::from_toml_str(
        r#"
        [[rule]]
        capability = "network.routeros.*"
        decision = "require_approval"
        "#,
    )
    .expect("policy parses");
    // Sanity: the write cap really does require approval under this policy.
    assert_eq!(
        policy
            .evaluate("network.routeros.firewall_add_drop_rule")
            .decision,
        Decision::RequireApproval,
        "guard policy must gate the write with require_approval — not weakened"
    );
    let approvals = ApprovalQueue::new(pool.clone());
    let guard = Arc::new(Guard::new(KillState::new(), policy, approvals));

    // Audit sink so broker.execute emits broker.execute events we can query.
    let audit: Arc<dyn AuditSink> = Arc::new(PostgresAuditSink::new(pool.clone()));

    // The production broker: inventory + real Postgres vault + audit + stub SSH
    // transport, then `.with_guard(...)`. This is the SAME assembly path
    // `build_production_broker` uses — guard gate → vault resolve → transport
    // dispatch → audit event, exactly what a chat/plan write hits in prod.
    let broker = Arc::new(
        Broker::with_production_admin(inventory.clone(), vault.clone(), audit.clone(), transports)
            .with_guard(guard.clone()),
    );

    // Spawn the REAL RouterOS driver under the supervisor — same as main.rs.
    let routeros = Arc::new(RouterOsDriver::new(
        AgentId::new("agent:routeros"),
        broker.clone(),
        "agent:routeros",
    ));
    supervisor
        .spawn(routeros.clone() as Arc<dyn Agent>)
        .await
        .expect("spawn routeros driver");
    // Let the supervised runner subscribe to the broadcast before we dispatch.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Dispatcher over the SAME bus+registry; orchestrator with recording memory.
    let dispatcher = Dispatcher::new(bus.clone(), registry.clone());
    let memory = Arc::new(RecordingMemory::default());
    let service = OrchestratorService::new(pool.clone(), dispatcher)
        .with_memory(memory.clone() as Arc<dyn MemoryService>);

    // (3) Create the 2-step plan -------------------------------------------------
    //   step0 = firewall_add_drop_rule (WRITE, compensator firewall_remove_rule),
    //           a real CIDR + comment, target://mikrotik-edge.
    //   step1 = a bogus capability that no agent provides → dispatch fails closed
    //           (DispatchError::CapabilityNotFound) AFTER step0 succeeds.
    let steps = vec![
        StepDef {
            capability_name: "network.routeros.firewall_add_drop_rule".into(),
            capability_version: "1.0.0".into(),
            target_ref: Some(target.clone()),
            credential_ref: None,
            params: serde_json::json!({
                "dst_address": "185.60.216.0/24",
                "comment": "e2e-block",
            }),
            depends_on_index: vec![],
        },
        StepDef {
            // Fails closed at dispatch: no agent provides this capability, so
            // require_by_capability returns CapabilityNotFound → step Failed.
            capability_name: "network.routeros.does_not_exist".into(),
            capability_version: "1.0.0".into(),
            target_ref: Some(target.clone()),
            credential_ref: None,
            params: serde_json::json!({}),
            depends_on_index: vec![0],
        },
    ];
    let plan = service
        .create_plan(None, "e2e remediation vertical: block 185.60.216.0/24 then fail", steps)
        .await
        .expect("create_plan");
    println!("[plan] created plan {} with 2 steps", plan.id);

    // (4) Background operator: approve every pending approval as it appears.
    //     Uses the REAL ApprovalQueue::decide (status→approved, decided_by).
    let operator_pool = pool.clone();
    let operator = tokio::spawn(async move {
        let q = ApprovalQueue::new(operator_pool);
        let mut approved = 0usize;
        // Run for the duration of the plan; each pending row is approved once.
        for _ in 0..600 {
            match q.list_pending(16).await {
                Ok(pending) => {
                    for rec in pending {
                        match q.decide(rec.id, operator_user, ApprovalStatus::Approved).await {
                            Ok(updated) => {
                                approved += 1;
                                println!(
                                    "[operator] APPROVED approval {} for `{}` (#{approved})",
                                    updated.id, updated.capability
                                );
                            }
                            Err(e) => {
                                // Lost a race (already decided) — ignore.
                                println!("[operator] decide {} skipped: {e}", rec.id);
                            }
                        }
                    }
                }
                Err(e) => println!("[operator] list_pending error: {e}"),
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        approved
    });

    // (5) Run the plan. The forward write parks on approval → operator approves
    //     → add executes → step1 fails → saga compensates step0 (remove parks on
    //     approval → operator approves → remove executes) → Failed→Incident.
    println!("[run] run_plan starting …");
    let final_status = service
        .run_plan(plan.id, "agent:e2e")
        .await
        .expect("run_plan");
    println!("[run] run_plan finished with status {final_status:?}");

    operator.abort();

    // (6) ASSERT each hop from Postgres + the recording stubs -------------------

    // -- plan final status == Failed
    assert_eq!(
        final_status,
        PlanStatus::Failed,
        "plan must fail-stop after step1 fails"
    );
    let plan_row = service.get_plan(plan.id).await.expect("get_plan").expect("plan exists");
    assert_eq!(plan_row.status, PlanStatus::Failed, "persisted plan status");

    // -- step statuses: step0 Compensated, step1 Failed
    let persisted_steps = service.list_steps(plan.id).await.expect("list_steps");
    assert_eq!(persisted_steps.len(), 2, "two steps");
    let step0 = &persisted_steps[0];
    let step1 = &persisted_steps[1];
    println!(
        "[assert] step0 `{}` -> {:?} ; step1 `{}` -> {:?}",
        step0.capability_name, step0.status, step1.capability_name, step1.status
    );
    // step0 reached Succeeded (the approved write executed) THEN Compensated.
    assert_eq!(
        step0.status,
        StepStatus::Compensated,
        "step0 must end Compensated (it succeeded, then saga rolled it back)"
    );
    assert_eq!(step1.status, StepStatus::Failed, "step1 must be Failed");

    // -- StubTransport recorded BOTH the add and the remove commands.
    let records = ssh_transport.records().await;
    println!("[assert] StubTransport recorded {} op(s):", records.len());
    for r in &records {
        if let daimon_transport::Op::ShellCommand { command, .. } = &r.op {
            println!("           - {command}");
        }
    }
    let commands: Vec<String> = records
        .iter()
        .filter_map(|r| match &r.op {
            daimon_transport::Op::ShellCommand { command, .. } => Some(command.clone()),
            _ => None,
        })
        .collect();
    assert!(
        commands.iter().any(|c| c.contains("firewall filter add")
            && c.contains("dst-address=185.60.216.0/24")),
        "StubTransport must have executed the firewall ADD; got {commands:?}"
    );
    assert!(
        commands.iter().any(|c| c.contains("firewall filter remove")),
        "StubTransport must have executed the compensating firewall REMOVE; got {commands:?}"
    );

    // -- audit.events contains a broker.execute for BOTH the add and the remove
    //    (both writes went through the guarded broker path). The broker carries
    //    the PostgresAuditSink, so each broker.execute emitted a row; we query
    //    for BrokerExecute events against the target and split by op_summary.
    let filter = AuditFilter {
        actor_id: None,
        action: Some(ActionKind::BrokerExecute),
        target_ref: Some(target.clone()), // ILIKE %target% — per-run unique
        result: None,
        since: None,
        until: None,
    };
    let events = audit
        .query(&filter, 100, 0)
        .await
        .expect("audit query");
    let op_summaries: Vec<String> = events
        .iter()
        .filter_map(|e| e.op_summary.clone())
        .collect();
    println!("[assert] audit broker.execute op_summaries: {op_summaries:?}");
    assert!(
        op_summaries.iter().any(|s| s.contains("filter add")),
        "audit must contain a broker.execute for firewall_add_drop_rule; got {op_summaries:?}"
    );
    assert!(
        op_summaries.iter().any(|s| s.contains("filter remove")),
        "audit must contain a broker.execute for firewall_remove_rule; got {op_summaries:?}"
    );

    // -- approvals table shows exactly 2 approved decisions (add + remove), both
    //    decided_by the operator user.
    // Scope to THIS plan: the orchestrator stamps each write's actor as
    // `orchestrator:<plan_id>:<step_id>[:compensate]`, so filtering on the plan
    // id prefix isolates this run's approvals from any prior run's rows.
    let actor_prefix = format!("orchestrator:{}:%", plan.id);
    let approved_rows = pool
        .get()
        .await
        .unwrap()
        .query(
            "SELECT capability, decided_by FROM public.approvals
             WHERE status = 'approved' AND actor_id LIKE $1
             ORDER BY created_at ASC",
            &[&actor_prefix],
        )
        .await
        .expect("query approvals");
    let approved_caps: Vec<String> = approved_rows.iter().map(|r| r.get::<_, String>(0)).collect();
    println!("[assert] approved approvals: {approved_caps:?}");
    assert_eq!(
        approved_rows.len(),
        2,
        "exactly 2 approvals approved (the add + the compensating remove); got {approved_caps:?}"
    );
    for row in &approved_rows {
        let decided_by: Option<Uuid> = row.get(1);
        assert_eq!(
            decided_by,
            Some(operator_user),
            "each approval must be decided by the operator user (real decision-record)"
        );
    }
    assert!(
        approved_caps.iter().any(|c| c == "network.routeros.firewall_add_drop_rule"),
        "the ADD write was approval-gated + approved"
    );
    assert!(
        approved_caps.iter().any(|c| c == "network.routeros.firewall_remove_rule"),
        "the compensating REMOVE write was approval-gated + approved"
    );

    // -- the recording MemoryService captured exactly one Incident record.
    let captured = memory.captured();
    println!(
        "[assert] memory captured {} record(s): {:?}",
        captured.len(),
        captured.iter().map(|r| r.body.kind()).collect::<Vec<_>>()
    );
    assert_eq!(captured.len(), 1, "exactly one terminal record captured");
    assert_eq!(
        captured[0].body.kind(),
        RecordKind::Incident,
        "a Failed plan must capture an Incident"
    );

    println!("[done] remediation vertical PASSED — detect→approve→add→fail→rollback-approve→remove→incident");
}
