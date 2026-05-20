use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::RwLock;

use crate::refspec::TargetRef;
use crate::target::{ManagedTarget, TargetKind, TargetMetadata};

/// Inventory of managed targets. **Internal — broker-only.**
///
/// Two read surfaces:
/// - Public-ish (broker calls on behalf of agents): `get_metadata`, `list`.
///   Returns `TargetMetadata`, never the credential ref.
/// - Broker-only: `get_managed` returns the full `ManagedTarget` including
///   `credential_ref`. Used by broker.execute() to look up the credential
///   before invoking transport.
#[async_trait]
pub trait Inventory: Send + Sync {
    /// Metadata view — safe to expose via broker to agents.
    async fn get_metadata(&self, r#ref: &TargetRef) -> Result<TargetMetadata, InventoryError>;

    /// Full target record including credential ref. Broker-only.
    async fn get_managed(&self, r#ref: &TargetRef) -> Result<ManagedTarget, InventoryError>;

    /// List metadata for all targets, optionally filtered by kind.
    async fn list(&self, kind_filter: Option<TargetKind>) -> Vec<TargetMetadata>;

    /// Register a new target.
    async fn upsert(&self, target: ManagedTarget) -> Result<(), InventoryError>;

    /// Remove a target.
    async fn remove(&self, r#ref: &TargetRef) -> Result<(), InventoryError>;
}

#[derive(Debug, Error)]
pub enum InventoryError {
    #[error("target `{0}` not found in inventory")]
    NotFound(String),
    #[error("{0}")]
    Other(String),
}

/// In-memory inventory impl. Phase 2 default; Phase 3 will add a SQLite-backed
/// variant via `daimon-memory`.
#[derive(Default, Clone)]
pub struct InMemoryRegistry {
    inner: Arc<RwLock<HashMap<TargetRef, ManagedTarget>>>,
}

impl InMemoryRegistry {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Inventory for InMemoryRegistry {
    async fn get_metadata(&self, r#ref: &TargetRef) -> Result<TargetMetadata, InventoryError> {
        self.inner
            .read()
            .await
            .get(r#ref)
            .map(|t| t.metadata())
            .ok_or_else(|| InventoryError::NotFound(r#ref.to_string()))
    }

    async fn get_managed(&self, r#ref: &TargetRef) -> Result<ManagedTarget, InventoryError> {
        self.inner
            .read()
            .await
            .get(r#ref)
            .cloned()
            .ok_or_else(|| InventoryError::NotFound(r#ref.to_string()))
    }

    async fn list(&self, kind_filter: Option<TargetKind>) -> Vec<TargetMetadata> {
        let guard = self.inner.read().await;
        guard
            .values()
            .filter(|t| kind_filter.map_or(true, |k| t.kind == k))
            .map(|t| t.metadata())
            .collect()
    }

    async fn upsert(&self, target: ManagedTarget) -> Result<(), InventoryError> {
        self.inner.write().await.insert(target.r#ref.clone(), target);
        Ok(())
    }

    async fn remove(&self, r#ref: &TargetRef) -> Result<(), InventoryError> {
        self.inner
            .write()
            .await
            .remove(r#ref)
            .map(|_| ())
            .ok_or_else(|| InventoryError::NotFound(r#ref.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::{TargetKind, TransportKind};
    use std::collections::BTreeMap;

    fn sample(name: &str, kind: TargetKind) -> ManagedTarget {
        ManagedTarget {
            r#ref: TargetRef::parse(&format!("target://{name}")).unwrap(),
            kind,
            transport: TransportKind::Ssh,
            host: format!("{name}.local"),
            port: 22,
            credential_ref: format!("vault://infra/{name}"),
            labels: BTreeMap::new(),
            capabilities: vec![],
        }
    }

    #[tokio::test]
    async fn upsert_then_get_managed_returns_full_record() {
        let reg = InMemoryRegistry::new();
        let t = sample("mikrotik-edge", TargetKind::Network);
        reg.upsert(t.clone()).await.unwrap();
        let got = reg.get_managed(&t.r#ref).await.unwrap();
        assert_eq!(got, t);
        assert_eq!(got.credential_ref, "vault://infra/mikrotik-edge");
    }

    #[tokio::test]
    async fn get_metadata_omits_credential_ref() {
        let reg = InMemoryRegistry::new();
        let t = sample("mikrotik-edge", TargetKind::Network);
        reg.upsert(t.clone()).await.unwrap();
        let md = reg.get_metadata(&t.r#ref).await.unwrap();
        let json = serde_json::to_string(&md).unwrap();
        assert!(!json.contains("vault://"));
        assert!(!json.contains("credential_ref"));
    }

    #[tokio::test]
    async fn list_filters_by_kind() {
        let reg = InMemoryRegistry::new();
        reg.upsert(sample("mikrotik-edge", TargetKind::Network))
            .await
            .unwrap();
        reg.upsert(sample("nargothrond", TargetKind::Host)).await.unwrap();
        reg.upsert(sample("nargothrond-pve", TargetKind::Platform))
            .await
            .unwrap();

        let nets = reg.list(Some(TargetKind::Network)).await;
        assert_eq!(nets.len(), 1);
        let all = reg.list(None).await;
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn get_missing_returns_not_found() {
        let reg = InMemoryRegistry::new();
        let r = TargetRef::parse("target://missing").unwrap();
        let err = reg.get_managed(&r).await.unwrap_err();
        assert!(matches!(err, InventoryError::NotFound(_)));
    }
}
