//! P2 commit 3 — the Phase-8 "Block TikTok on VLAN 20" locked vertical, ported
//! from `daimon-tool-network` to the `RouterOsDriver` reference driver.
//!
//! Proves the whole action path through the DRIVER (not the old NetworkAgent):
//!
//! ```text
//! operator intent → RouterOsDriver::remediate(firewall_add_drop_rule, params)
//!                 → build_command (param::validate chokepoint)
//!                 → Broker.execute → Vault.resolve(cred://) → Transport.execute
//!                 → OpResult → Receipt
//! ```
//!
//! Plus the bus-adapter path (`impl Agent::handle`) proving:
//!   - a `NetworkRequest` decodes and dispatches to the right command,
//!   - the reply is a well-formed `NetworkResponse`,
//!   - `env.from` (the caller) is carried as the ExecRequest actor (FR-HAR-17)
//!     — verified at the pure `build_exec_request` seam in the crate unit tests.
//!
//! With StubTransport the dispatch comes back ok; the assertion is that the
//! recorded transport call carries the correct RouterOS command and that the
//! credential's plaintext bytes were not exposed to the driver (D19).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use daimon_broker::Broker;
use daimon_core::{
    Agent, AgentContext, AgentEnvelope, AgentId, BusHandle, CoreError, Recipient,
};
use daimon_driver::{Driver, TargetClass};
use daimon_driver_firewall_routeros::{NetworkRequest, NetworkResponse, RouterOsDriver};
use daimon_inventory::{
    InMemoryRegistry, Inventory, ManagedTarget, TargetKind, TargetRef as InvTargetRef,
    TransportKind,
};
use daimon_transport::{StubTransport, Transport};
use daimon_vault::{Credential, CredentialRef, StubVaultClient};
use serde_json::json;
use tokio::sync::Mutex;

async fn make_wiring() -> (Arc<Broker>, Arc<StubTransport>) {
    let inv = Arc::new(InMemoryRegistry::new());
    let vault = Arc::new(StubVaultClient::new());
    let stub_ssh = Arc::new(StubTransport::new("ssh"));

    let cred_ref = CredentialRef::parse("vault://mikrotik-edge-ssh").unwrap();
    vault
        .insert(
            cred_ref.clone(),
            Credential::SshPassword {
                username: "admin".into(),
                password: "stub-secret-payload".into(),
            },
        )
        .await;

    inv.upsert(ManagedTarget {
        r#ref: InvTargetRef::parse("target://mikrotik-edge").unwrap(),
        kind: TargetKind::Network,
        transport: TransportKind::Ssh,
        host: "10.0.0.1".into(),
        port: 22,
        credential_ref: cred_ref.to_string(),
        labels: BTreeMap::new(),
        capabilities: vec!["network.routeros.firewall_add_drop_rule".into()],
    })
    .await
    .unwrap();

    let mut transports: HashMap<TransportKind, Arc<dyn Transport>> = HashMap::new();
    transports.insert(TransportKind::Ssh, stub_ssh.clone());

    let broker = Arc::new(Broker::new(inv, vault, transports));
    (broker, stub_ssh)
}

fn driver(broker: Arc<Broker>) -> RouterOsDriver {
    RouterOsDriver::new(
        AgentId::new("driver-firewall-routeros-test"),
        broker,
        "agent:network:test",
    )
}

/// A one-shot bus handle that captures the single reply the adapter sends.
struct CapturingBus {
    reply: Mutex<Option<AgentEnvelope>>,
}

impl CapturingBus {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            reply: Mutex::new(None),
        })
    }
    async fn take(&self) -> Option<AgentEnvelope> {
        self.reply.lock().await.take()
    }
}

#[async_trait]
impl BusHandle for CapturingBus {
    async fn send(&self, env: AgentEnvelope) -> Result<(), CoreError> {
        *self.reply.lock().await = Some(env);
        Ok(())
    }
}

#[tokio::test]
async fn driver_class_is_firewall() {
    let (broker, _) = make_wiring().await;
    assert_eq!(driver(broker).class(), TargetClass::Firewall);
}

#[tokio::test]
async fn remediate_block_tiktok_dispatches_correct_command() {
    let (broker, stub_ssh) = make_wiring().await;
    let d = driver(broker);

    let receipt = d
        .remediate(
            "target://mikrotik-edge",
            "network.routeros.firewall_add_drop_rule",
            json!({
                "dst_address": "185.60.216.0/24",
                "in_interface": "vlan20",
                "comment": "block-tiktok-phase8-demo"
            }),
        )
        .await
        .expect("remediate ok");
    assert_eq!(receipt.capability, "network.routeros.firewall_add_drop_rule");

    // Verify the transport saw the exact CLI command we wanted to ship.
    let records = stub_ssh.records().await;
    assert_eq!(records.len(), 1, "exactly one transport call");
    let cmd = match &records[0].op {
        daimon_transport::Op::ShellCommand { command, .. } => command.clone(),
        other => panic!("unexpected op: {other:?}"),
    };
    assert!(
        cmd.starts_with("/ip firewall filter add chain=forward action=drop"),
        "command missing chain+action prefix: {cmd}"
    );
    assert!(cmd.contains("dst-address=185.60.216.0/24"), "dst missing: {cmd}");
    assert!(cmd.contains("in-interface=vlan20"), "iface missing: {cmd}");
    assert!(
        cmd.contains("comment=\"block-tiktok-phase8-demo\""),
        "comment missing: {cmd}"
    );

    // D19 boundary check — the transport received a real credential (non-zero
    // size); the driver never saw the plaintext.
    assert!(
        records[0].secret_byte_count > 0,
        "transport got an empty credential"
    );
}

#[tokio::test]
async fn remediate_rejects_shell_metachars_via_param_validate() {
    let (broker, _) = make_wiring().await;
    let d = driver(broker);

    let err = d
        .remediate(
            "target://mikrotik-edge",
            "network.routeros.firewall_add_drop_rule",
            json!({ "dst_address": "victim; /system shutdown" }),
        )
        .await
        .expect_err("must reject metachars");
    assert!(
        format!("{err}").contains("disallowed char"),
        "expected disallowed-char error, got: {err}"
    );
}

#[tokio::test]
async fn read_state_interface_list_dispatches_read_command() {
    let (broker, stub_ssh) = make_wiring().await;
    let d = driver(broker);

    let doc = d
        .read_state("target://mikrotik-edge", json!({ "table": "interface" }))
        .await
        .expect("read_state ok");
    assert_eq!(doc.target, "target://mikrotik-edge");

    let records = stub_ssh.records().await;
    assert_eq!(records.len(), 1);
    match &records[0].op {
        daimon_transport::Op::ShellCommand { command, .. } => {
            assert_eq!(command, "/interface print");
        }
        other => panic!("unexpected op: {other:?}"),
    }
}

#[tokio::test]
async fn diagnose_surfaces_finding_suggesting_remove_rule() {
    let (broker, _) = make_wiring().await;
    let d = driver(broker);

    let findings = d
        .diagnose("target://mikrotik-edge", "traffic to blocked host still flowing")
        .await
        .expect("diagnose ok");
    assert_eq!(findings.len(), 1);
    // The driver surfaces a candidate but does NOT decide the fix — suggested
    // capability points at firewall_remove_rule with empty params.
    assert_eq!(
        findings[0].suggested_capability.as_deref(),
        Some("network.routeros.firewall_remove_rule")
    );
}

#[tokio::test]
async fn bus_adapter_decodes_request_and_replies() {
    let (broker, stub_ssh) = make_wiring().await;
    let d = driver(broker);
    let bus = CapturingBus::new();
    let ctx = AgentContext::new(AgentId::new("driver-firewall-routeros-test"), bus.clone());

    // The envelope's `from` is the real caller — FR-HAR-17 says this becomes the
    // ExecRequest actor. (The verbatim propagation is asserted in the crate's
    // build_exec_request unit test; here we drive the full handle() path.)
    let env = AgentEnvelope::new(
        AgentId::new("user:carol"),
        Recipient::ByCapability {
            name: "network.routeros.firewall_add_drop_rule".into(),
            version_req: "^1".parse().unwrap(),
        },
        serde_json::to_value(NetworkRequest {
            capability: "network.routeros.firewall_add_drop_rule".into(),
            target_ref: "target://mikrotik-edge".into(),
            timeout_secs: Some(15),
            params: Some(json!({ "dst_address": "10.0.0.0/24" })),
        })
        .unwrap(),
    );

    d.handle(env, ctx).await.expect("handle ok");

    let reply = bus.take().await.expect("adapter replied");
    let resp: NetworkResponse = serde_json::from_value(reply.body).expect("NetworkResponse");
    assert!(resp.success, "dispatch should succeed: {:?}", resp.error);
    let out = resp.output.expect("output present");
    assert!(out.command.contains("dst-address=10.0.0.0/24"));

    // Command reached the transport.
    let records = stub_ssh.records().await;
    assert_eq!(records.len(), 1);
}

#[tokio::test]
async fn bus_adapter_rejects_bad_body_with_error_reply() {
    let (broker, _) = make_wiring().await;
    let d = driver(broker);
    let bus = CapturingBus::new();
    let ctx = AgentContext::new(AgentId::new("driver-firewall-routeros-test"), bus.clone());

    let env = AgentEnvelope::new(
        AgentId::new("user:dave"),
        Recipient::Direct(AgentId::new("driver-firewall-routeros-test")),
        json!({ "not": "a network request" }),
    );
    d.handle(env, ctx).await.expect("handle ok");

    let reply = bus.take().await.expect("adapter replied");
    let resp: NetworkResponse = serde_json::from_value(reply.body).expect("NetworkResponse");
    assert!(!resp.success);
    assert!(resp.error.unwrap().contains("invalid request"));
}
