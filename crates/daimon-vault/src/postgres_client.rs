//! Postgres-backed `VaultClient` impl (Phase 2c D3b).
//!
//! Storage: one row per credential in `vault.credentials`. The payload is
//! serde_json-serialized `Credential`, then sealed via XChaCha20-Poly1305
//! with the master key. Each row carries its own nonce inside the sealed
//! blob. JSON (not bincode) because Credential uses
//! `#[serde(tag = "kind")]` internally-tagged enums which bincode cannot
//! deserialize. Payload is encrypted anyway — JSON's verbosity costs ~3-5x
//! more disk per row but the rows are small (~hundreds of bytes each).
//!
//! Concurrency: tokio-postgres async + deadpool pool. No mutex; Postgres
//! handles concurrency internally. The 5-minute TTL LRU cache lives on a
//! tokio::sync::Mutex.
//!
//! Single-org: credentials are org-wide (name unique across the vault).

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use daimon_db::Pool;
use lru::LruCache;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;
use tracing::{debug, instrument, warn};
use uuid::Uuid;

use crate::client::{VaultClient, VaultError};
use crate::credential::{Credential, CredentialKind};
use crate::master_key::MasterKey;
use crate::refspec::CredentialRef;
use crate::sealed::SealedSession;

const DEFAULT_CACHE_SIZE: usize = 64;
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMetadata {
    pub id: Uuid,
    pub name: String,
    pub kind: CredentialKind,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct PostgresVaultClient {
    inner: Arc<Inner>,
}

struct Inner {
    pool: Pool,
    sealer: SealedSession,
    cache: AsyncMutex<LruCache<String, CacheEntry>>,
    cache_ttl: Duration,
}

#[derive(Clone)]
struct CacheEntry {
    cred: Credential,
    inserted_at: Instant,
}

impl PostgresVaultClient {
    /// Construct with a live `daimon-db` pool and the master DEK. Master key
    /// is dropped after `SealedSession` derives its cipher state.
    pub fn new(pool: Pool, master_key: MasterKey) -> Self {
        let sealer = SealedSession::from_key(master_key.as_bytes());
        Self {
            inner: Arc::new(Inner {
                pool,
                sealer,
                cache: AsyncMutex::new(LruCache::new(
                    NonZeroUsize::new(DEFAULT_CACHE_SIZE).expect("non-zero"),
                )),
                cache_ttl: DEFAULT_CACHE_TTL,
            }),
        }
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn list_metadata(&self) -> Result<Vec<CredentialMetadata>, VaultError> {
        let client = self
            .inner
            .pool
            .get()
            .await
            .map_err(|e| VaultError::Transport(format!("pool: {e}")))?;
        let rows = client
            .query(
                "SELECT id, name, kind, created_at, updated_at
                 FROM vault.credentials
                 ORDER BY name ASC",
                &[],
            )
            .await
            .map_err(|e| VaultError::Transport(format!("list: {e}")))?;
        rows.into_iter()
            .map(|r| {
                let kind_str: String = r.get(2);
                let kind = parse_kind(&kind_str)?;
                Ok(CredentialMetadata {
                    id: r.get(0),
                    name: r.get(1),
                    kind,
                    created_at: r.get(3),
                    updated_at: r.get(4),
                })
            })
            .collect()
    }

    #[instrument(skip(self, cred), level = "debug")]
    pub async fn create(&self, name: &str, cred: Credential) -> Result<Uuid, VaultError> {
        let kind = cred.kind();
        let blob = serde_json::to_vec(&cred)
            .map_err(|e| VaultError::Other(format!("serialize: {e}")))?;
        let sealed = self
            .inner
            .sealer
            .seal(&blob)
            .map_err(|e| VaultError::Other(format!("seal: {e}")))?;
        let client = self
            .inner
            .pool
            .get()
            .await
            .map_err(|e| VaultError::Transport(format!("pool: {e}")))?;
        let row = client
            .query_one(
                "INSERT INTO vault.credentials
                    (name, kind, payload_sealed, encryption_version)
                 VALUES ($1, $2, $3, 1)
                 RETURNING id",
                &[&name, &kind_to_str(&kind), &sealed],
            )
            .await
            .map_err(|e| VaultError::Transport(format!("insert: {e}")))?;
        Ok(row.get(0))
    }

    #[instrument(skip(self, cred), level = "debug")]
    pub async fn update(&self, id: Uuid, cred: Credential) -> Result<(), VaultError> {
        let kind = cred.kind();
        let blob = serde_json::to_vec(&cred)
            .map_err(|e| VaultError::Other(format!("serialize: {e}")))?;
        let sealed = self
            .inner
            .sealer
            .seal(&blob)
            .map_err(|e| VaultError::Other(format!("seal: {e}")))?;
        let client = self
            .inner
            .pool
            .get()
            .await
            .map_err(|e| VaultError::Transport(format!("pool: {e}")))?;
        let n = client
            .execute(
                "UPDATE vault.credentials
                 SET kind = $1, payload_sealed = $2
                 WHERE id = $3",
                &[&kind_to_str(&kind), &sealed, &id],
            )
            .await
            .map_err(|e| VaultError::Transport(format!("update: {e}")))?;
        if n == 0 {
            return Err(VaultError::NotFound(id.to_string()));
        }
        self.invalidate_cache_for_id(id).await;
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn rename(&self, id: Uuid, new_name: &str) -> Result<(), VaultError> {
        let client = self
            .inner
            .pool
            .get()
            .await
            .map_err(|e| VaultError::Transport(format!("pool: {e}")))?;
        let n = client
            .execute(
                "UPDATE vault.credentials
                 SET name = $1
                 WHERE id = $2",
                &[&new_name, &id],
            )
            .await
            .map_err(|e| VaultError::Transport(format!("rename: {e}")))?;
        if n == 0 {
            return Err(VaultError::NotFound(id.to_string()));
        }
        self.invalidate_cache_for_id(id).await;
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn delete(&self, id: Uuid) -> Result<(), VaultError> {
        let client = self
            .inner
            .pool
            .get()
            .await
            .map_err(|e| VaultError::Transport(format!("pool: {e}")))?;
        let n = client
            .execute(
                "DELETE FROM vault.credentials WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| VaultError::Transport(format!("delete: {e}")))?;
        if n == 0 {
            return Err(VaultError::NotFound(id.to_string()));
        }
        self.invalidate_cache_for_id(id).await;
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn reveal(&self, id: Uuid) -> Result<Credential, VaultError> {
        let client = self
            .inner
            .pool
            .get()
            .await
            .map_err(|e| VaultError::Transport(format!("pool: {e}")))?;
        let row = client
            .query_opt(
                "SELECT payload_sealed FROM vault.credentials
                 WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| VaultError::Transport(format!("reveal: {e}")))?
            .ok_or_else(|| VaultError::NotFound(id.to_string()))?;
        let sealed: Vec<u8> = row.get(0);
        let blob = self
            .inner
            .sealer
            .unseal(&sealed)
            .map_err(|e| VaultError::Other(format!("unseal: {e}")))?;
        let cred: Credential = serde_json::from_slice(&blob)
            .map_err(|e| VaultError::Other(format!("decode: {e}")))?;
        Ok(cred)
    }

    async fn invalidate_cache_for_id(&self, _id: Uuid) {
        // The cache key is the credential name (resolve() lookups go by ref).
        // After id-targeted mutation we don't know which name maps to which
        // id without a query — clear the whole cache. For a 64-entry LRU
        // this is cheap.
        self.inner.cache.lock().await.clear();
    }
}

#[async_trait]
impl VaultClient for PostgresVaultClient {
    async fn resolve(&self, vref: &CredentialRef) -> Result<Credential, VaultError> {
        let name = vref.name().ok_or_else(|| VaultError::NotFound(vref.to_string()))?;

        // Cache hit?
        {
            let mut cache = self.inner.cache.lock().await;
            if let Some(entry) = cache.get(name) {
                if entry.inserted_at.elapsed() < self.inner.cache_ttl {
                    debug!(name, "vault cache hit");
                    return Ok(entry.cred.clone());
                } else {
                    cache.pop(name);
                }
            }
        }

        // Miss: hit Postgres.
        let client = self
            .inner
            .pool
            .get()
            .await
            .map_err(|e| VaultError::Transport(format!("pool: {e}")))?;
        let row = client
            .query_opt(
                "SELECT payload_sealed FROM vault.credentials
                 WHERE name = $1",
                &[&name],
            )
            .await
            .map_err(|e| VaultError::Transport(format!("resolve: {e}")))?
            .ok_or_else(|| VaultError::NotFound(vref.to_string()))?;
        let sealed: Vec<u8> = row.get(0);
        let blob = self
            .inner
            .sealer
            .unseal(&sealed)
            .map_err(|e| VaultError::Other(format!("unseal: {e}")))?;
        let cred: Credential = serde_json::from_slice(&blob)
            .map_err(|e| VaultError::Other(format!("decode: {e}")))?;

        // Cache.
        self.inner.cache.lock().await.put(
            name.to_string(),
            CacheEntry {
                cred: cred.clone(),
                inserted_at: Instant::now(),
            },
        );
        Ok(cred)
    }
}

fn kind_to_str(k: &CredentialKind) -> &'static str {
    match k {
        CredentialKind::SshKey => "SshKey",
        CredentialKind::SshPassword => "SshPassword",
        CredentialKind::ApiToken => "ApiToken",
        CredentialKind::Generic => "Generic",
    }
}

fn parse_kind(s: &str) -> Result<CredentialKind, VaultError> {
    match s {
        "SshKey" => Ok(CredentialKind::SshKey),
        "SshPassword" => Ok(CredentialKind::SshPassword),
        "ApiToken" => Ok(CredentialKind::ApiToken),
        "Generic" => Ok(CredentialKind::Generic),
        other => {
            warn!(kind = other, "unknown CredentialKind in DB");
            Err(VaultError::Decode(format!("unknown kind: {other}")))
        }
    }
}
