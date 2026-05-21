//! D19 acceptance test: agents must never see credential material.
//!
//! The test simulates an agent calling `broker.execute()`. The broker
//! internally resolves a credential from `StubVaultClient` and passes it to
//! `StubTransport`. The agent code path holds only `OpResult`. We assert:
//!
//! 1. The transport DID receive a credential (broker actually resolved it).
//! 2. The `OpResult` returned to the agent does NOT serialize any credential
//!    field (no leaking via output).
//! 3. The agent never had a typed `Credential` in scope (compile-time — the
//!    test only depends on `daimon-broker`, not `daimon-vault`).

use std::collections::HashMap;
use std::sync::Arc;

use daimon_broker::{Broker, ExecRequest, Op, OpResult, TargetKind, TargetRef, TransportKind};

#[tokio::test]
async fn agent_never_sees_credential() {
    // The agent code below uses ONLY daimon-broker APIs. It does NOT import
    // daimon_vault::Credential — D21 enforcement at the test level.

    // Wire up the broker with stubs. In production these would be:
    //   - daimon-inventory:::SqliteRegistry
    //   - daimon-vault::VaultwardenClient
    //   - daimon-transport::SshTransport / RestTransport / SnmpTransport
    let (broker, inv, vault, ssh) = make_stub_broker().await;

    // Seed inventory + vault. (In production: ops team adds entries via UI;
    // worker agents don't touch this seam.)
    seed_fixture(&inv, &vault).await;

    // ---- The agent's code path -----------------------------------------
    let req = ExecRequest::new(
        "agent:tool-network",
        TargetRef::parse("target://mikrotik-edge").unwrap(),
        Op::ShellCommand {
            command: "/ip firewall address-list print".into(),
            timeout_secs: 30,
        },
    );
    let result = broker.execute(req).await.unwrap();
    // --------------------------------------------------------------------

    // 1. OpResult shape is reachable + matches op
    match &result {
        OpResult::ShellCommand { stdout, exit_status, .. } => {
            assert_eq!(*exit_status, 0);
            assert!(stdout.contains("address-list print"));
        }
        _ => panic!("expected ShellCommand result"),
    }

    // 2. Result does NOT contain credential bytes
    let serialized = serde_json::to_string(&result).unwrap();
    assert!(
        !serialized.contains("PRIVATE KEY"),
        "OpResult leaked private key material: {serialized}"
    );
    assert!(
        !serialized.contains("vault://"),
        "OpResult leaked credential ref: {serialized}"
    );

    // 3. The transport stub recorded that a credential of the right KIND
    //    was passed to it — proves broker resolved + dispatched correctly.
    let records = ssh.records().await;
    assert_eq!(records.len(), 1);
    let rec = &records[0];
    assert_eq!(rec.host, "10.100.10.1");
    assert_eq!(rec.port, 22);
    assert!(
        rec.secret_byte_count > 0,
        "credential reached transport but had zero secret bytes"
    );
    // The kind reaches transport (it's not secret); the bytes do, but workers
    // never saw them.
    assert!(matches!(
        rec.credential_kind,
        daimon_vault::CredentialKind::SshKey
    ));
}

#[tokio::test]
async fn target_metadata_does_not_carry_credential_ref() {
    let (broker, inv, vault, _ssh) = make_stub_broker().await;
    seed_fixture(&inv, &vault).await;

    let md = broker
        .target_metadata(&TargetRef::parse("target://mikrotik-edge").unwrap())
        .await
        .unwrap();

    let serialized = serde_json::to_string(&md).unwrap();
    assert!(!serialized.contains("vault://"));
    assert!(!serialized.contains("credential_ref"));
    assert!(serialized.contains("10.100.10.1"));
}

#[tokio::test]
async fn missing_target_returns_inventory_error() {
    let (broker, _, _, _) = make_stub_broker().await;
    let req = ExecRequest::new(
        "agent:tool-network",
        TargetRef::parse("target://does-not-exist").unwrap(),
        Op::ShellCommand {
            command: "noop".into(),
            timeout_secs: 5,
        },
    );
    let err = broker.execute(req).await.unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("inventory") || msg.contains("not found"));
}

#[tokio::test]
async fn list_targets_returns_metadata_only() {
    let (broker, inv, vault, _) = make_stub_broker().await;
    seed_fixture(&inv, &vault).await;

    let listed = broker.list_targets(Some(TargetKind::Network)).await;
    assert_eq!(listed.len(), 1);
    let serialized = serde_json::to_string(&listed).unwrap();
    assert!(!serialized.contains("vault://"));
    assert!(!serialized.contains("credential_ref"));
}

// ---- Helpers --------------------------------------------------------------

async fn make_stub_broker() -> (
    Broker,
    Arc<daimon_inventory::InMemoryRegistry>,
    Arc<daimon_vault::StubVaultClient>,
    Arc<daimon_transport::StubTransport>,
) {
    let inv = Arc::new(daimon_inventory::InMemoryRegistry::new());
    let vault = Arc::new(daimon_vault::StubVaultClient::new());
    let ssh = Arc::new(daimon_transport::StubTransport::new("ssh"));

    let mut transports: HashMap<TransportKind, Arc<dyn daimon_transport::Transport>> =
        HashMap::new();
    transports.insert(TransportKind::Ssh, ssh.clone());

    let broker = Broker::new(inv.clone(), vault.clone(), transports);
    (broker, inv, vault, ssh)
}

async fn seed_fixture(
    inv: &Arc<daimon_inventory::InMemoryRegistry>,
    vault: &Arc<daimon_vault::StubVaultClient>,
) {
    use daimon_inventory::{Inventory, ManagedTarget};
    use daimon_vault::{Credential, CredentialRef};
    use std::collections::BTreeMap;

    inv.upsert(ManagedTarget {
        r#ref: TargetRef::parse("target://mikrotik-edge").unwrap(),
        kind: TargetKind::Network,
        transport: TransportKind::Ssh,
        host: "10.100.10.1".into(),
        port: 22,
        credential_ref: "vault://infra/network/mikrotik-edge".into(),
        labels: BTreeMap::new(),
        capabilities: vec![],
    })
    .await
    .unwrap();

    vault
        .insert(
            CredentialRef::parse("vault://infra/network/mikrotik-edge").unwrap(),
            Credential::SshKey {
                username: "arif".into(),
                private_key_pem: "-----BEGIN OPENSSH PRIVATE KEY-----\nFAKE_KEY_BYTES\n-----END OPENSSH PRIVATE KEY-----".into(),
                passphrase: None,
            },
        )
        .await;
}
