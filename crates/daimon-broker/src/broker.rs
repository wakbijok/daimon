use std::collections::HashMap;
use std::sync::Arc;

use daimon_audit::AuditSink;
use daimon_inventory::{Inventory, InventoryError, TargetMetadata, TargetRef, TransportKind};
use daimon_transport::{OpResult, Transport, TransportError, TransportTarget};
use daimon_vault::{
    CredentialRef, RefParseError as CredRefParseError, SqliteVaultClient, VaultClient, VaultError,
};
use thiserror::Error;
use tracing::{debug, instrument};

use crate::request::ExecRequest;

/// The action broker (D19).
///
/// Agents call `execute(req)` and receive only an `OpResult`. Internally:
/// 1. Look up the target in inventory → get host + port + transport_kind + credential_ref
/// 2. Resolve the credential_ref via vault → typed `Credential`
/// 3. Dispatch via the registered `Transport` for that kind, passing the credential by reference
/// 4. Drop the credential (zeroized via the `Credential` type's `ZeroizeOnDrop`)
/// 5. Return `OpResult` to the agent
///
/// The credential is in scope only for steps 3–4 and never enters worker memory.
///
/// Admin proxy methods (D22/D23/D24 — `vault_*`, `inventory_*`, `audit_*`)
/// live in `daimon-broker/src/admin.rs`. They require the production
/// constructor `with_production_admin` which wires up `vault_admin` (concrete
/// `SqliteVaultClient`) and `audit` (the structured event sink). The legacy
/// `new()` constructor leaves admin disabled (stub usage / tests).
pub struct Broker {
    pub(crate) inventory: Arc<dyn Inventory>,
    pub(crate) vault: Arc<dyn VaultClient>,
    pub(crate) vault_admin: Option<Arc<SqliteVaultClient>>,
    pub(crate) audit: Option<Arc<dyn AuditSink>>,
    pub(crate) transports: HashMap<TransportKind, Arc<dyn Transport>>,
}

impl Broker {
    /// Legacy / test constructor — admin proxy disabled.
    pub fn new(
        inventory: Arc<dyn Inventory>,
        vault: Arc<dyn VaultClient>,
        transports: HashMap<TransportKind, Arc<dyn Transport>>,
    ) -> Self {
        Self {
            inventory,
            vault,
            vault_admin: None,
            audit: None,
            transports,
        }
    }

    /// Production constructor (D22 in-tree vault + D23 audit log + D24 admin).
    ///
    /// `vault` is the concrete `Arc<SqliteVaultClient>` — used both as the
    /// `dyn VaultClient` for the agent `execute` resolve path AND as the
    /// admin-CRUD handle. `audit` is the append-only event sink — every
    /// state-changing admin call emits an event.
    pub fn with_production_admin(
        inventory: Arc<dyn Inventory>,
        vault: Arc<SqliteVaultClient>,
        audit: Arc<dyn AuditSink>,
        transports: HashMap<TransportKind, Arc<dyn Transport>>,
    ) -> Self {
        let vault_dyn: Arc<dyn VaultClient> = vault.clone();
        Self {
            inventory,
            vault: vault_dyn,
            vault_admin: Some(vault),
            audit: Some(audit),
            transports,
        }
    }

    /// Agent-safe metadata lookup. Returns target info WITHOUT credential ref.
    pub async fn target_metadata(&self, r#ref: &TargetRef) -> Result<TargetMetadata, BrokerError> {
        self.inventory
            .get_metadata(r#ref)
            .await
            .map_err(BrokerError::from)
    }

    /// Agent-safe target listing.
    pub async fn list_targets(
        &self,
        kind_filter: Option<daimon_inventory::TargetKind>,
    ) -> Vec<TargetMetadata> {
        self.inventory.list(kind_filter).await
    }

    /// The protected operation: resolve target, resolve credential, dispatch
    /// transport, zeroize credential, return result. Credentials never leak.
    /// Emits a single audit event (`broker.execute`) capturing the whole
    /// flow — actor, target, credential ref, op summary, result, latency
    /// (D23). On error, the failure stage is recorded in the metadata.
    #[instrument(skip(self, req), fields(target = %req.target_ref, actor = %req.actor_id))]
    pub async fn execute(&self, req: ExecRequest) -> Result<OpResult, BrokerError> {
        let start = std::time::Instant::now();
        let result = self.execute_inner(&req).await;
        self.audit_execute(&req, &result, start).await;
        result
    }

    async fn execute_inner(&self, req: &ExecRequest) -> Result<OpResult, BrokerError> {
        // Step 1: inventory lookup (broker-only view).
        let managed = self
            .inventory
            .get_managed(&req.target_ref)
            .await
            .map_err(BrokerError::from)?;

        // Step 2: parse the credential ref and resolve via vault.
        let vref = CredentialRef::parse(&managed.credential_ref)
            .map_err(BrokerError::CredentialRefParse)?;
        let credential = self.vault.resolve(&vref).await.map_err(BrokerError::from)?;

        // Step 3: dispatch transport. `credential` is consumed by reference
        // here; the `Credential` type is `ZeroizeOnDrop`, so the bytes are
        // wiped when this function returns (or panics).
        let transport = self
            .transports
            .get(&managed.transport)
            .ok_or_else(|| BrokerError::NoTransport(managed.transport))?;

        let target = TransportTarget {
            host: managed.host.clone(),
            port: managed.port,
        };

        debug!(
            transport = transport.id(),
            cred_kind = ?credential.kind(),
            "dispatching op"
        );

        // Step 4 + 5: execute, then `credential` drops + zeroizes when this
        // scope exits. `OpResult` does not carry credential material.
        let result = transport
            .execute(&target, &req.op, &credential)
            .await
            .map_err(BrokerError::from)?;

        Ok(result)
    }

    /// Emit one structured audit event per broker.execute call.
    /// Captures op kind, target, and result. Failure stage and message
    /// land in metadata for forensic queries.
    async fn audit_execute(
        &self,
        req: &ExecRequest,
        result: &Result<OpResult, BrokerError>,
        start: std::time::Instant,
    ) {
        let Some(sink) = self.audit.as_ref() else {
            // No audit configured (Broker::new without admin wiring) — silent.
            return;
        };

        // Look up the credential ref the broker resolved against — best-effort,
        // tolerate inventory misses (the error case already records it).
        let credential_ref = self
            .inventory
            .get_managed(&req.target_ref)
            .await
            .ok()
            .map(|m| m.credential_ref);

        let res_tag = match result {
            Ok(_) => daimon_audit::AuditResult::Success,
            Err(_) => daimon_audit::AuditResult::Error,
        };

        let op_summary = op_summary_for(&req.op);

        let mut ev = daimon_audit::NewAuditEvent::new(
            req.actor_id.clone(),
            daimon_audit::ActionKind::BrokerExecute,
            res_tag,
        )
        .with_target(req.target_ref.to_string())
        .with_op_summary(op_summary)
        .with_latency_ms(start.elapsed().as_millis() as u64);

        if let Some(c) = credential_ref {
            ev = ev.with_credential(c);
        }
        if let Err(e) = result {
            ev = ev.with_metadata("error", format!("{e}"));
        }

        if let Err(emit_err) = sink.append(ev).await {
            tracing::warn!(error = %emit_err, "broker.execute audit emit failed");
        }
    }
}

fn op_summary_for(op: &daimon_transport::Op) -> String {
    use daimon_transport::Op;
    match op {
        Op::ShellCommand { command, .. } => {
            let truncated: String = command.chars().take(80).collect();
            format!("ssh:exec:{}", truncated)
        }
        Op::Http { method, path, .. } => format!("http:{method:?}:{path}"),
        Op::SnmpGet { oid } => format!("snmp:get:{oid}"),
        Op::SnmpSet { oid, .. } => format!("snmp:set:{oid}"),
        Op::SnmpWalk { oid_root } => format!("snmp:walk:{oid_root}"),
    }
}

#[derive(Debug, Error)]
pub enum BrokerError {
    #[error("inventory: {0}")]
    Inventory(#[from] InventoryError),
    #[error("vault: {0}")]
    Vault(#[from] VaultError),
    #[error("transport: {0}")]
    Transport(#[from] TransportError),
    #[error("invalid credential_ref in inventory entry: {0}")]
    CredentialRefParse(CredRefParseError),
    #[error("no transport registered for kind `{0:?}`")]
    NoTransport(TransportKind),
    #[error("admin backend `{0}` not available — broker constructed without production admin wiring")]
    AdminBackendNotAvailable(&'static str),
    #[error("audit: {0}")]
    Audit(String),
}

/// Helper for tests + Phase 2 prototyping — wires a Broker with stub
/// implementations of inventory/vault/transport without needing real
/// Vaultwarden or SSH.
#[cfg(test)]
pub mod test_helpers {
    use super::*;
    use daimon_inventory::InMemoryRegistry;
    use daimon_transport::StubTransport;
    use daimon_vault::StubVaultClient;

    pub fn stub_broker() -> (
        Broker,
        Arc<InMemoryRegistry>,
        Arc<StubVaultClient>,
        Arc<StubTransport>,
    ) {
        let inv = Arc::new(InMemoryRegistry::new());
        let vault = Arc::new(StubVaultClient::new());
        let ssh = Arc::new(StubTransport::new("ssh"));

        let mut transports: HashMap<TransportKind, Arc<dyn Transport>> = HashMap::new();
        transports.insert(TransportKind::Ssh, ssh.clone());

        let broker = Broker::new(inv.clone(), vault.clone(), transports);
        (broker, inv, vault, ssh)
    }
}
