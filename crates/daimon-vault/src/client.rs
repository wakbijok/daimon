use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::RwLock;

use crate::credential::Credential;
use crate::refspec::CredentialRef;

/// Trait the rest of daimon talks to when it wants a credential.
///
/// Two impls today: `StubVaultClient` for development and tests (hardcoded
/// credentials), and (next session) `VaultwardenClient` for the real
/// Vaultwarden REST API.
///
/// `VaultClient` is `Send + Sync` so it can be stored in `AgentContext` or
/// shared across tokio tasks.
#[async_trait]
pub trait VaultClient: Send + Sync {
    /// Resolve a credential reference to a live `Credential`.
    async fn resolve(&self, vref: &CredentialRef) -> Result<Credential, VaultError>;
}

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("credential ref `{0}` not found")]
    NotFound(String),
    #[error("vault session is locked or expired")]
    SessionLocked,
    #[error("vault transport error: {0}")]
    Transport(String),
    #[error("vault response decode error: {0}")]
    Decode(String),
    #[error("{0}")]
    Other(String),
}

/// Development / test impl. Holds a map from `CredentialRef` (as string) to
/// `Credential` and serves them directly. Useful for transport development
/// and integration tests without standing up Vaultwarden.
#[derive(Default, Clone)]
pub struct StubVaultClient {
    inner: Arc<RwLock<HashMap<String, Credential>>>,
}

impl StubVaultClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(&self, vref: CredentialRef, cred: Credential) {
        self.inner.write().await.insert(vref.to_string(), cred);
    }
}

#[async_trait]
impl VaultClient for StubVaultClient {
    async fn resolve(&self, vref: &CredentialRef) -> Result<Credential, VaultError> {
        self.inner
            .read()
            .await
            .get(&vref.to_string())
            .cloned()
            .ok_or_else(|| VaultError::NotFound(vref.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential::CredentialKind;

    #[tokio::test]
    async fn stub_resolves_inserted_ref() {
        let stub = StubVaultClient::new();
        let vref =
            CredentialRef::parse("vault://infra/network/mikrotik-edge").unwrap();
        stub.insert(
            vref.clone(),
            Credential::SshKey {
                username: "arif".into(),
                private_key_pem: "fake-key-content".into(),
                passphrase: None,
            },
        )
        .await;
        let resolved = stub.resolve(&vref).await.unwrap();
        assert_eq!(resolved.kind(), CredentialKind::SshKey);
    }

    #[tokio::test]
    async fn stub_returns_not_found_for_missing_ref() {
        let stub = StubVaultClient::new();
        let vref = CredentialRef::parse("vault://infra/network/missing").unwrap();
        let err = stub.resolve(&vref).await.unwrap_err();
        assert!(matches!(err, VaultError::NotFound(_)));
    }
}
