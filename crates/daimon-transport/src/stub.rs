use std::sync::Arc;

use async_trait::async_trait;
use daimon_vault::{Credential, CredentialKind};
use tokio::sync::Mutex;

use crate::op::{Op, OpResult, TransportError};
use crate::transport::{Transport, TransportTarget};

/// What `StubTransport` recorded about a single call. Used by tests to assert
/// the broker passed the right credential KIND (not its bytes — those are
/// only-seen-by-test-via-side-channel) and that it actually invoked the
/// transport.
#[derive(Debug, Clone)]
pub struct StubTransportRecord {
    pub host: String,
    pub port: u16,
    pub op: Op,
    pub credential_kind: CredentialKind,
    /// Length of secret material in the credential — proves the credential
    /// arrived non-empty without leaking bytes. Tests that want stronger
    /// assertions can use a custom transport.
    pub secret_byte_count: usize,
}

/// In-memory transport that captures calls instead of doing I/O.
///
/// Phase 2 uses this to test the broker end-to-end without spinning up an
/// actual SSH or REST endpoint. Phase 2 continuation replaces with real
/// `russh` / `reqwest` / `csnmp` impls.
#[derive(Clone, Default)]
pub struct StubTransport {
    id: String,
    records: Arc<Mutex<Vec<StubTransportRecord>>>,
}

impl StubTransport {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            records: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn records(&self) -> Vec<StubTransportRecord> {
        self.records.lock().await.clone()
    }
}

#[async_trait]
impl Transport for StubTransport {
    fn id(&self) -> &str {
        &self.id
    }

    async fn execute(
        &self,
        target: &TransportTarget,
        op: &Op,
        cred: &Credential,
    ) -> Result<OpResult, TransportError> {
        let secret_byte_count = match cred {
            Credential::SshKey {
                private_key_pem,
                passphrase,
                ..
            } => private_key_pem.len() + passphrase.as_deref().map(str::len).unwrap_or(0),
            Credential::SshPassword { password, .. } => password.len(),
            Credential::ApiToken { token } => token.len(),
            Credential::Generic { fields } => fields.values().map(|v| v.len()).sum(),
        };

        let record = StubTransportRecord {
            host: target.host.clone(),
            port: target.port,
            op: op.clone(),
            credential_kind: cred.kind(),
            secret_byte_count,
        };
        self.records.lock().await.push(record);

        // Return a canned ok-result matching the op's shape.
        Ok(match op {
            Op::ShellCommand { command, .. } => OpResult::ShellCommand {
                stdout: format!("[stub] ran: {command}"),
                stderr: String::new(),
                exit_status: 0,
            },
            Op::Http { .. } => OpResult::Http {
                status: 200,
                body: serde_json::json!({"stub": true}),
                headers: Default::default(),
            },
            Op::SnmpGet { .. } | Op::SnmpWalk { .. } => OpResult::Snmp {
                values: Default::default(),
            },
            Op::SnmpSet { .. } => OpResult::SnmpSetAck,
        })
    }
}
