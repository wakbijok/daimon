//! P2 commit 9 — the generic `ConnectorDriver` end-to-end.
//!
//! Proves, mirroring the RouterOS reference-driver test approach:
//!
//! 1. **Profiles load + project** — `k8s.toml` loads from a dir and its
//!    `[[capability]]` blocks project into registered `Capability`s.
//! 2. **REST reachability** — a READ (`orchestrator.k8s.pod.status`) dispatched
//!    against a wiremock REST server comes back as a doc (real `RestTransport`,
//!    real bearer-token auth, real HTTP round-trip).
//! 3. **Injection chokepoint** — a crafted value in a templated param is rejected
//!    by `param::validate` BEFORE any `Op` is built.
//! 4. **Write → guard** — a WRITE (`orchestrator.k8s.deploy.restart`) reaches the
//!    Guard's policy (denied by a test policy → proves the SAME guard path a
//!    real approval would traverse).
//! 5. **Second-driver routing (AC-P2-10)** — over ONE bus + registry, a dispatch
//!    by `orchestrator.k8s.*` routes to the ConnectorDriver (REST) while
//!    `network.routeros.*` routes to the RouterOS driver (SSH). By-capability
//!    routing across two transports + two classes = the platform-agnostic point.
//!
//! D21 note: this TEST wires vault/inventory/transport directly. That is legal —
//! the D21 invariant applies to PRODUCTION deps only (tests are never linked into
//! the driver binary), exactly as the RouterOS driver's e2e test does.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use daimon_broker::Broker;
use daimon_core::AgentId;
use daimon_driver::connector::ConnectorProfile;
use daimon_driver::{ConnectorDriver, Driver};
use daimon_inventory::{
    InMemoryRegistry, Inventory, ManagedTarget, TargetKind, TargetRef as InvTargetRef,
    TransportKind,
};
use daimon_runtime::{CapabilityRegistry, Dispatcher, InProcBus, Supervisor};
use daimon_transport::{RestTransport, StubTransport, Transport};
use daimon_vault::{Credential, CredentialRef, StubVaultClient};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// -------------------------------------------------------------------------
// Wiring helpers
// -------------------------------------------------------------------------

const K8S_TOML: &str = include_str!("../../../deploy/connectors/k8s.toml");

/// Build a broker with a REST transport pointing at nothing in particular; the
/// caller seeds the inventory target host/port. `guard_policy` (if Some) attaches
/// a Guard with that policy TOML — used for the write→guard test.
async fn broker_with_rest(
    guard_policy: Option<&str>,
) -> (Arc<Broker>, Arc<InMemoryRegistry>, Arc<StubVaultClient>) {
    let inv = Arc::new(InMemoryRegistry::new());
    let vault = Arc::new(StubVaultClient::new());
    let rest: Arc<dyn Transport> = Arc::new(RestTransport::new());
    let mut transports: HashMap<TransportKind, Arc<dyn Transport>> = HashMap::new();
    transports.insert(TransportKind::Rest, rest);

    let mut broker = Broker::new(inv.clone(), vault.clone(), transports);
    if let Some(policy_toml) = guard_policy {
        let policy = daimon_guard::PolicyEngine::from_toml_str(policy_toml).expect("policy parses");
        let pool = daimon_db::build_pool("postgres://test@localhost:5432/test").expect("lazy pool");
        let approvals = daimon_guard::ApprovalQueue::new(pool);
        let guard = Arc::new(daimon_guard::Guard::new(
            daimon_guard::KillState::new(),
            policy,
            approvals,
        ));
        broker = broker.with_guard(guard);
    }
    (Arc::new(broker), inv, vault)
}

/// Seed a REST target (k8s API) with a bearer ApiToken credential.
async fn seed_k8s_target(inv: &InMemoryRegistry, vault: &StubVaultClient, host: &str, port: u16) {
    let cred_ref = CredentialRef::parse("vault://k3s-sa-token").unwrap();
    vault
        .insert(
            cred_ref.clone(),
            Credential::ApiToken {
                token: "sa-bearer-token".into(),
            },
        )
        .await;
    inv.upsert(ManagedTarget {
        r#ref: InvTargetRef::parse("target://k3s-lab").unwrap(),
        kind: TargetKind::Platform,
        transport: TransportKind::Rest,
        host: host.to_string(),
        port,
        credential_ref: cred_ref.to_string(),
        labels: BTreeMap::new(),
        capabilities: vec!["orchestrator.k8s.pod.status".into()],
    })
    .await
    .unwrap();
}

/// Parse the shipped k8s profile and rewrite each op path to an ABSOLUTE URL at
/// the wiremock base, so the bare `/api/...` template hits plaintext wiremock
/// (RestTransport composes `https://` for bare paths — production is https).
/// `{param}` slots survive the rewrite and are still substituted at dispatch.
fn k8s_profile_at_base(base: &str) -> ConnectorProfile {
    let mut profile: ConnectorProfile = toml::from_str(K8S_TOML).expect("k8s.toml parses");
    for cap in &mut profile.capabilities {
        cap.op.path = format!("{base}{}", cap.op.path);
    }
    profile
}

fn connector(broker: Arc<Broker>, profiles: Vec<ConnectorProfile>) -> ConnectorDriver {
    ConnectorDriver::from_profiles(
        AgentId::new("agent:connector:test"),
        broker,
        profiles,
        "agent:connector:test",
    )
}

// -------------------------------------------------------------------------
// 1. Profiles load + project
// -------------------------------------------------------------------------

#[tokio::test]
async fn from_dir_loads_shipped_profiles_and_projects_caps() {
    let (broker, _, _) = broker_with_rest(None).await;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/connectors");
    let driver = ConnectorDriver::from_dir(
        AgentId::new("agent:connector"),
        broker,
        &dir,
        "agent:connector",
    )
    .expect("load ok")
    .expect("dir present + non-empty");

    let names: Vec<&str> = driver.capabilities().iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"orchestrator.k8s.pod.status"));
    assert!(names.contains(&"orchestrator.k8s.deploy.restart"));
    assert!(names.contains(&"orchestrator.k8s.deploy.rollback"));
    assert_eq!(driver.class(), daimon_driver::TargetClass::Orchestrator);
}

#[tokio::test]
async fn from_dir_absent_returns_none() {
    let (broker, _, _) = broker_with_rest(None).await;
    let missing = std::path::Path::new("/nonexistent/daimon/connectors/dir");
    let got =
        ConnectorDriver::from_dir(AgentId::new("agent:connector"), broker, missing, "a").unwrap();
    assert!(got.is_none(), "absent dir must skip gracefully (None)");
}

// -------------------------------------------------------------------------
// 2. REST reachability via wiremock
// -------------------------------------------------------------------------

#[tokio::test]
async fn read_dispatched_against_wiremock_returns_doc() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/default/pods/web-0"))
        .and(header("authorization", "Bearer sa-bearer-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "Pod",
            "metadata": { "name": "web-0", "namespace": "default" },
            "status": { "phase": "Running" }
        })))
        .mount(&server)
        .await;

    // host/port from wiremock; base URL baked into the profile op paths.
    let uri = server.uri();
    let stripped = uri.strip_prefix("http://").unwrap();
    let (host, port) = stripped.split_once(':').unwrap();

    let (broker, inv, vault) = broker_with_rest(None).await;
    seed_k8s_target(&inv, &vault, host, port.parse().unwrap()).await;
    let d = connector(broker, vec![k8s_profile_at_base(&uri)]);

    let doc = d
        .read_state(
            "target://k3s-lab",
            json!({
                "capability": "orchestrator.k8s.pod.status",
                "params": { "namespace": "default", "name": "web-0" }
            }),
        )
        .await
        .expect("read_state ok");

    assert_eq!(doc.target, "target://k3s-lab");
    assert_eq!(doc.doc["status"], 200);
    assert_eq!(doc.doc["result"]["status"]["phase"], "Running");
    assert_eq!(doc.doc["result"]["metadata"]["name"], "web-0");
}

// -------------------------------------------------------------------------
// 3. Injection chokepoint (param::validate before any Op)
// -------------------------------------------------------------------------

#[tokio::test]
async fn injection_in_templated_param_rejected_before_broker() {
    // A StubTransport records every Op it sees. If the chokepoint works, the
    // crafted value is rejected and the transport records ZERO calls.
    let inv = Arc::new(InMemoryRegistry::new());
    let vault = Arc::new(StubVaultClient::new());
    let stub_rest = Arc::new(StubTransport::new("rest"));
    let mut transports: HashMap<TransportKind, Arc<dyn Transport>> = HashMap::new();
    transports.insert(TransportKind::Rest, stub_rest.clone());
    let broker = Arc::new(Broker::new(inv.clone(), vault.clone(), transports));
    seed_k8s_target(&inv, &vault, "127.0.0.1", 6443).await;

    let d = connector(broker, vec![toml::from_str(K8S_TOML).unwrap()]);

    // `namespace` is declared `identifier`; `default/../../secret` breaks it.
    let err = d
        .read_state(
            "target://k3s-lab",
            json!({
                "capability": "orchestrator.k8s.pod.status",
                "params": { "namespace": "default/../../secret", "name": "web-0" }
            }),
        )
        .await
        .expect_err("must reject the path-traversal namespace");
    assert!(
        format!("{err}").contains("disallowed char"),
        "expected disallowed-char rejection, got: {err}"
    );

    // No Op ever reached the transport — the chokepoint fired before dispatch.
    assert!(
        stub_rest.records().await.is_empty(),
        "a rejected param must never reach the transport"
    );
}

// -------------------------------------------------------------------------
// 4. Write reaches the Guard (policy)
// -------------------------------------------------------------------------

#[tokio::test]
async fn write_reaches_guard_policy() {
    // A policy that DENIES the write proves the guard is on the path — the guard
    // returns PolicyDenied before touching the approval queue (so no Postgres).
    // require_approval would instead park on the queue; deny is the queue-free
    // proof that the SAME guard gate is traversed.
    let policy = r#"
        [[rule]]
        capability = "orchestrator.k8s.*"
        decision = "deny"
    "#;
    let (broker, inv, vault) = broker_with_rest(Some(policy)).await;
    seed_k8s_target(&inv, &vault, "127.0.0.1", 6443).await;
    let d = connector(broker, vec![toml::from_str(K8S_TOML).unwrap()]);

    let err = d
        .remediate(
            "target://k3s-lab",
            "orchestrator.k8s.deploy.restart",
            json!({ "namespace": "default", "name": "web" }),
        )
        .await
        .expect_err("policy denies the write");
    let msg = format!("{err}");
    assert!(
        msg.contains("policy") || msg.contains("denies") || msg.contains("deny"),
        "write must be gated by the guard policy, got: {msg}"
    );
}

#[tokio::test]
async fn read_bypasses_guard_policy() {
    // The read cap must bypass policy even under an all-deny guard (server-side
    // read-only derivation). We point it at a wiremock so the read succeeds.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/default/pods/web-0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": { "phase": "Running" } })))
        .mount(&server)
        .await;
    let uri = server.uri();
    let stripped = uri.strip_prefix("http://").unwrap();
    let (host, port) = stripped.split_once(':').unwrap();

    let deny_all = r#"
        [[rule]]
        capability = "orchestrator.k8s.*"
        decision = "deny"
    "#;
    let (broker, inv, vault) = broker_with_rest(Some(deny_all)).await;
    seed_k8s_target(&inv, &vault, host, port.parse().unwrap()).await;
    let d = connector(broker, vec![k8s_profile_at_base(&uri)]);

    let doc = d
        .read_state(
            "target://k3s-lab",
            json!({
                "capability": "orchestrator.k8s.pod.status",
                "params": { "namespace": "default", "name": "web-0" }
            }),
        )
        .await
        .expect("read must bypass the deny policy (server-derived read-only)");
    assert_eq!(doc.doc["result"]["status"]["phase"], "Running");
}

// -------------------------------------------------------------------------
// 5. Second-driver routing (AC-P2-10)
// -------------------------------------------------------------------------

#[tokio::test]
async fn second_driver_routing_across_transports() {
    use daimon_driver_firewall_routeros::RouterOsDriver;

    // ---- shared broker: BOTH SSH (routeros) + REST (k8s) transports. ----
    let inv = Arc::new(InMemoryRegistry::new());
    let vault = Arc::new(StubVaultClient::new());
    let stub_ssh = Arc::new(StubTransport::new("ssh"));
    let stub_rest = Arc::new(StubTransport::new("rest"));
    let mut transports: HashMap<TransportKind, Arc<dyn Transport>> = HashMap::new();
    transports.insert(TransportKind::Ssh, stub_ssh.clone());
    transports.insert(TransportKind::Rest, stub_rest.clone());

    // SSH target (routeros) + REST target (k8s).
    let ssh_cred = CredentialRef::parse("vault://mikrotik-ssh").unwrap();
    vault
        .insert(
            ssh_cred.clone(),
            Credential::SshPassword {
                username: "admin".into(),
                password: "pw".into(),
            },
        )
        .await;
    inv.upsert(ManagedTarget {
        r#ref: InvTargetRef::parse("target://mikrotik-edge").unwrap(),
        kind: TargetKind::Network,
        transport: TransportKind::Ssh,
        host: "10.0.0.1".into(),
        port: 22,
        credential_ref: ssh_cred.to_string(),
        labels: BTreeMap::new(),
        capabilities: vec![],
    })
    .await
    .unwrap();
    let broker = Arc::new(Broker::new(inv.clone(), vault.clone(), transports));
    seed_k8s_target(&inv, &vault, "127.0.0.1", 6443).await;

    // ---- ONE bus + registry + supervisor; spawn BOTH drivers. ----
    let bus = InProcBus::new();
    let registry = CapabilityRegistry::new();
    let supervisor = Arc::new(Supervisor::new(bus.clone(), registry.clone()));

    let routeros = Arc::new(RouterOsDriver::new(
        AgentId::new("agent:routeros"),
        broker.clone(),
        "agent:routeros",
    ));
    let connector_driver = Arc::new(
        ConnectorDriver::from_profiles(
            AgentId::new("agent:connector"),
            broker.clone(),
            vec![toml::from_str(K8S_TOML).unwrap()],
            "agent:connector",
        ),
    );
    supervisor
        .spawn(routeros as Arc<dyn daimon_core::Agent>)
        .await
        .expect("spawn routeros");
    supervisor
        .spawn(connector_driver as Arc<dyn daimon_core::Agent>)
        .await
        .expect("spawn connector");
    // Let both supervised runners subscribe before we dispatch.
    tokio::time::sleep(Duration::from_millis(150)).await;

    let dispatcher = Dispatcher::new(bus, registry);

    // ---- dispatch a network.routeros.* READ → must route to the SSH driver. ----
    let ros_req: semver::VersionReq = "^1".parse().unwrap();
    let ros_reply = dispatcher
        .dispatch(
            AgentId::new("user:test"),
            "network.routeros.interface_list",
            &ros_req,
            json!({
                "capability": "network.routeros.interface_list",
                "target_ref": "target://mikrotik-edge"
            }),
            Duration::from_secs(3),
        )
        .await
        .expect("routeros dispatch");
    assert_eq!(ros_reply["success"], true, "routeros reply: {ros_reply:?}");

    // ---- dispatch an orchestrator.k8s.* READ → must route to the REST connector. ----
    let k8s_req: semver::VersionReq = "^1".parse().unwrap();
    let k8s_reply = dispatcher
        .dispatch(
            AgentId::new("user:test"),
            "orchestrator.k8s.pod.status",
            &k8s_req,
            json!({
                "capability": "orchestrator.k8s.pod.status",
                "target_ref": "target://k3s-lab",
                "params": { "namespace": "default", "name": "web-0" }
            }),
            Duration::from_secs(3),
        )
        .await
        .expect("k8s dispatch");
    assert_eq!(k8s_reply["success"], true, "k8s reply: {k8s_reply:?}");

    // ---- the two dispatches hit DIFFERENT transports (SSH vs REST). ----
    let ssh_records = stub_ssh.records().await;
    let rest_records = stub_rest.records().await;
    assert_eq!(ssh_records.len(), 1, "routeros must have used the SSH transport once");
    assert_eq!(rest_records.len(), 1, "k8s must have used the REST transport once");

    // SSH call carried a ShellCommand; REST call carried an Http op — proof the
    // SAME by-capability dispatch fanned out across two transports + two classes.
    assert!(matches!(
        ssh_records[0].op,
        daimon_transport::Op::ShellCommand { .. }
    ));
    assert!(matches!(rest_records[0].op, daimon_transport::Op::Http { .. }));
}
