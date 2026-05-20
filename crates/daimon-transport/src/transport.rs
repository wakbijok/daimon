use async_trait::async_trait;
use daimon_vault::Credential;

use crate::op::{Op, OpResult, TransportError};

/// The connection target — host + port — for a transport operation.
///
/// Note this is *not* `TargetRef` (that's broker/inventory's concern). By the
/// time transport runs, the broker has already resolved the ref to a concrete
/// host + port + credential.
#[derive(Debug, Clone)]
pub struct TransportTarget {
    pub host: String,
    pub port: u16,
}

/// The transport contract. Broker holds a registry of transports keyed by
/// `TransportKind` and dispatches based on the inventory entry.
///
/// `execute` takes a `Credential` by reference — transport implementations
/// MUST NOT clone it into long-lived state. The broker owns the credential
/// for exactly the duration of the call, then zeroizes (D19).
#[async_trait]
pub trait Transport: Send + Sync {
    /// Identifier for routing — `"ssh"`, `"rest"`, `"snmp"`, `"grpc"`.
    fn id(&self) -> &str;

    async fn execute(
        &self,
        target: &TransportTarget,
        op: &Op,
        cred: &Credential,
    ) -> Result<OpResult, TransportError>;
}
