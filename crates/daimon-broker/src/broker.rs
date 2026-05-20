use std::collections::HashMap;
use std::sync::Arc;

use daimon_inventory::{Inventory, InventoryError, TargetMetadata, TargetRef, TransportKind};
use daimon_transport::{OpResult, Transport, TransportError, TransportTarget};
use daimon_vault::{CredentialRef, RefParseError as CredRefParseError, VaultClient, VaultError};
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
pub struct Broker {
    inventory: Arc<dyn Inventory>,
    vault: Arc<dyn VaultClient>,
    transports: HashMap<TransportKind, Arc<dyn Transport>>,
}

impl Broker {
    pub fn new(
        inventory: Arc<dyn Inventory>,
        vault: Arc<dyn VaultClient>,
        transports: HashMap<TransportKind, Arc<dyn Transport>>,
    ) -> Self {
        Self {
            inventory,
            vault,
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
    #[instrument(skip(self, req), fields(target = %req.target_ref))]
    pub async fn execute(&self, req: ExecRequest) -> Result<OpResult, BrokerError> {
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
