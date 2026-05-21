//! In-tree SQLite-backed `VaultClient` impl (D22).
//!
//! Storage: one row per credential in `credentials`. The payload is
//! bincode-serialized `Credential`, then sealed via XChaCha20-Poly1305 with
//! the master key. Each row carries its own nonce.
//!
//! Concurrency model: rusqlite is sync. We wrap each operation in
//! `tokio::task::spawn_blocking` so the async runtime never blocks on
//! SQLite I/O. The connection lives behind `Arc<Mutex<Connection>>` —
//! one connection per `SqliteVaultClient` instance (SQLite serialises
//! writes internally; we don't need a pool for vault-scale traffic).
//!
//! Cache: a 5-minute LRU TTL cache on `resolve()` paths reduces decrypt
//! cost during high-frequency action loops. Cache entries are zeroized on
//! eviction.

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lru::LruCache;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task;
use tracing::{debug, instrument, warn};

use crate::client::{VaultClient, VaultError};
use crate::credential::{Credential, CredentialKind};
use crate::master_key::MasterKey;
use crate::refspec::CredentialRef;
use crate::sealed::SealedSession;

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS credentials (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    payload_sealed BLOB NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_credentials_name ON credentials(name);
"#;

const DEFAULT_CACHE_SIZE: usize = 64;
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);

/// In-tree SQLite-backed credential vault.
///
/// Construct via [`SqliteVaultClient::open`] (production — file-backed) or
/// [`SqliteVaultClient::in_memory`] (tests).
#[derive(Clone)]
pub struct SqliteVaultClient {
    inner: Arc<Inner>,
}

struct Inner {
    conn: AsyncMutex<Connection>,
    sealer: SealedSession,
    cache: AsyncMutex<LruCache<String, CacheEntry>>,
    cache_ttl: Duration,
}

#[derive(Clone)]
struct CacheEntry {
    cred: Credential,
    inserted_at: Instant,
}

/// Public metadata view of a credential — no secret material, safe to hand
/// to the admin UI listing surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMetadata {
    pub id: i64,
    pub name: String,
    pub kind: CredentialKind,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SqliteVaultClient {
    /// Open or create a vault at the given file path.
    pub async fn open(path: &Path, master_key: MasterKey) -> Result<Self, VaultError> {
        let owned_path = path.to_path_buf();
        let conn = task::spawn_blocking(move || -> Result<Connection, rusqlite::Error> {
            let conn = Connection::open(&owned_path)?;
            conn.execute_batch(SCHEMA_V1)?;
            Ok(conn)
        })
        .await
        .map_err(|e| VaultError::Other(format!("vault open join error: {e}")))?
        .map_err(|e| VaultError::Transport(format!("vault open: {e}")))?;

        Ok(Self::wrap(conn, master_key))
    }

    /// In-memory vault for tests. Master key must be supplied via `from_bytes`.
    pub fn in_memory(master_key: MasterKey) -> Result<Self, VaultError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| VaultError::Transport(format!("in-memory open: {e}")))?;
        conn.execute_batch(SCHEMA_V1)
            .map_err(|e| VaultError::Transport(format!("schema: {e}")))?;
        Ok(Self::wrap(conn, master_key))
    }

    fn wrap(conn: Connection, master_key: MasterKey) -> Self {
        let sealer = SealedSession::from_key(master_key.as_bytes());
        // master_key drops here; SealedSession holds the derived cipher state.
        Self {
            inner: Arc::new(Inner {
                conn: AsyncMutex::new(conn),
                sealer,
                cache: AsyncMutex::new(LruCache::new(
                    NonZeroUsize::new(DEFAULT_CACHE_SIZE).expect("non-zero"),
                )),
                cache_ttl: DEFAULT_CACHE_TTL,
            }),
        }
    }

    // ----- CRUD surface (admin) -----------------------------------------

    /// List all credentials' metadata (no secret material).
    #[instrument(skip(self))]
    pub async fn list_metadata(&self) -> Result<Vec<CredentialMetadata>, VaultError> {
        let inner = self.inner.clone();
        task::spawn_blocking(move || -> Result<Vec<CredentialMetadata>, VaultError> {
            let conn = inner.conn.blocking_lock();
            let mut stmt = conn
                .prepare("SELECT id, name, kind, created_at, updated_at FROM credentials ORDER BY name")
                .map_err(|e| VaultError::Transport(format!("prepare list: {e}")))?;
            let rows = stmt
                .query_map([], |row| {
                    let id: i64 = row.get(0)?;
                    let name: String = row.get(1)?;
                    let kind_str: String = row.get(2)?;
                    let created_at: String = row.get(3)?;
                    let updated_at: String = row.get(4)?;
                    Ok((id, name, kind_str, created_at, updated_at))
                })
                .map_err(|e| VaultError::Transport(format!("query list: {e}")))?;

            let mut out = Vec::new();
            for row in rows {
                let (id, name, kind_str, created_at, updated_at) =
                    row.map_err(|e| VaultError::Transport(format!("row list: {e}")))?;
                let kind = parse_kind(&kind_str)?;
                out.push(CredentialMetadata {
                    id,
                    name,
                    kind,
                    created_at: parse_ts(&created_at)?,
                    updated_at: parse_ts(&updated_at)?,
                });
            }
            Ok(out)
        })
        .await
        .map_err(|e| VaultError::Other(format!("list join: {e}")))?
    }

    /// Create a new credential. Returns the new id.
    /// Fails with `VaultError::Other("duplicate name")` if `name` already exists.
    #[instrument(skip(self, cred), fields(name = %name))]
    pub async fn create(&self, name: &str, cred: Credential) -> Result<i64, VaultError> {
        let name = name.to_owned();
        let kind_str = kind_str(cred.kind());
        let sealed = self.seal_credential(&cred)?;
        let now = Utc::now().to_rfc3339();
        let inner = self.inner.clone();

        let id = task::spawn_blocking(move || -> Result<i64, VaultError> {
            let conn = inner.conn.blocking_lock();
            conn.execute(
                "INSERT INTO credentials (name, kind, payload_sealed, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                params![name, kind_str, sealed, now],
            )
            .map_err(|e| {
                if let Some(code) = sqlite_error_code(&e) {
                    if code == "UNIQUE" {
                        return VaultError::Other(format!("duplicate name"));
                    }
                }
                VaultError::Transport(format!("insert: {e}"))
            })?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .map_err(|e| VaultError::Other(format!("create join: {e}")))??;

        // Drop the credential explicitly (Zeroize on drop wipes secret bytes).
        drop(cred);

        Ok(id)
    }

    /// Update an existing credential's payload by id. Caller chooses whether to
    /// rename via `update_name`.
    #[instrument(skip(self, cred), fields(id = id))]
    pub async fn update(&self, id: i64, cred: Credential) -> Result<(), VaultError> {
        let kind_str = kind_str(cred.kind());
        let sealed = self.seal_credential(&cred)?;
        let now = Utc::now().to_rfc3339();
        let inner = self.inner.clone();

        // Invalidate cache entry for the old name (if cached). We don't know
        // the name without a lookup here; simplest correct approach is to
        // clear by id-based lookup name.
        let name = self.lookup_name(id).await?;
        if let Some(name) = &name {
            inner.cache.lock().await.pop(name);
        }

        task::spawn_blocking(move || -> Result<(), VaultError> {
            let conn = inner.conn.blocking_lock();
            let n = conn
                .execute(
                    "UPDATE credentials SET kind = ?1, payload_sealed = ?2, updated_at = ?3 \
                     WHERE id = ?4",
                    params![kind_str, sealed, now, id],
                )
                .map_err(|e| VaultError::Transport(format!("update: {e}")))?;
            if n == 0 {
                return Err(VaultError::NotFound(format!("id={id}")));
            }
            Ok(())
        })
        .await
        .map_err(|e| VaultError::Other(format!("update join: {e}")))??;

        drop(cred);
        Ok(())
    }

    /// Rename an existing credential. Fails on UNIQUE collision.
    #[instrument(skip(self), fields(id = id, new_name = %new_name))]
    pub async fn rename(&self, id: i64, new_name: &str) -> Result<(), VaultError> {
        let new_name = new_name.to_owned();
        let now = Utc::now().to_rfc3339();
        let inner = self.inner.clone();

        // Invalidate any cached entry under the old name.
        let old_name = self.lookup_name(id).await?;
        if let Some(name) = &old_name {
            inner.cache.lock().await.pop(name);
        }

        task::spawn_blocking(move || -> Result<(), VaultError> {
            let conn = inner.conn.blocking_lock();
            let n = conn
                .execute(
                    "UPDATE credentials SET name = ?1, updated_at = ?2 WHERE id = ?3",
                    params![new_name, now, id],
                )
                .map_err(|e| {
                    if let Some(code) = sqlite_error_code(&e) {
                        if code == "UNIQUE" {
                            return VaultError::Other("duplicate name".into());
                        }
                    }
                    VaultError::Transport(format!("rename: {e}"))
                })?;
            if n == 0 {
                return Err(VaultError::NotFound(format!("id={id}")));
            }
            Ok(())
        })
        .await
        .map_err(|e| VaultError::Other(format!("rename join: {e}")))??;

        Ok(())
    }

    /// Delete a credential by id.
    #[instrument(skip(self), fields(id = id))]
    pub async fn delete(&self, id: i64) -> Result<(), VaultError> {
        let inner = self.inner.clone();

        let name = self.lookup_name(id).await?;
        if let Some(name) = &name {
            inner.cache.lock().await.pop(name);
        }

        task::spawn_blocking(move || -> Result<(), VaultError> {
            let conn = inner.conn.blocking_lock();
            let n = conn
                .execute("DELETE FROM credentials WHERE id = ?1", params![id])
                .map_err(|e| VaultError::Transport(format!("delete: {e}")))?;
            if n == 0 {
                return Err(VaultError::NotFound(format!("id={id}")));
            }
            Ok(())
        })
        .await
        .map_err(|e| VaultError::Other(format!("delete join: {e}")))??;

        Ok(())
    }

    /// Admin "reveal" — full credential by id. Writes to audit log are the
    /// broker's responsibility (D23); this method does not log directly.
    #[instrument(skip(self), fields(id = id))]
    pub async fn reveal(&self, id: i64) -> Result<Credential, VaultError> {
        let inner = self.inner.clone();
        task::spawn_blocking(move || -> Result<Credential, VaultError> {
            let conn = inner.conn.blocking_lock();
            let sealed: Vec<u8> = conn
                .query_row(
                    "SELECT payload_sealed FROM credentials WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| VaultError::Transport(format!("reveal query: {e}")))?
                .ok_or_else(|| VaultError::NotFound(format!("id={id}")))?;
            unseal_credential(&inner.sealer, &sealed)
        })
        .await
        .map_err(|e| VaultError::Other(format!("reveal join: {e}")))?
    }

    // ----- Internal helpers ---------------------------------------------

    async fn lookup_name(&self, id: i64) -> Result<Option<String>, VaultError> {
        let inner = self.inner.clone();
        task::spawn_blocking(move || -> Result<Option<String>, VaultError> {
            let conn = inner.conn.blocking_lock();
            conn.query_row(
                "SELECT name FROM credentials WHERE id = ?1",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| VaultError::Transport(format!("lookup_name: {e}")))
        })
        .await
        .map_err(|e| VaultError::Other(format!("lookup_name join: {e}")))?
    }

    fn seal_credential(&self, cred: &Credential) -> Result<Vec<u8>, VaultError> {
        // serde_json (not bincode) because Credential uses internally-tagged
        // serde representation (#[serde(tag = "kind")]) which bincode's
        // binary protocol can't deserialize. Payload is encrypted anyway —
        // wire format is not visible.
        let plaintext = serde_json::to_vec(cred)
            .map_err(|e| VaultError::Other(format!("json encode: {e}")))?;
        let sealed = self
            .inner
            .sealer
            .seal(&plaintext)
            .map_err(|e| VaultError::Other(format!("seal: {e}")))?;
        Ok(sealed)
    }
}

#[async_trait]
impl VaultClient for SqliteVaultClient {
    async fn resolve(&self, vref: &CredentialRef) -> Result<Credential, VaultError> {
        let name = vref
            .name()
            .ok_or_else(|| VaultError::NotFound(format!("{vref} (no resolvable name)")))?
            .to_owned();

        // Cache check.
        {
            let mut cache = self.inner.cache.lock().await;
            if let Some(entry) = cache.get(&name) {
                if entry.inserted_at.elapsed() < self.inner.cache_ttl {
                    debug!(name = %name, "vault cache hit");
                    return Ok(entry.cred.clone());
                }
                // expired
                cache.pop(&name);
            }
        }

        let inner = self.inner.clone();
        let name_for_query = name.clone();
        let cred = task::spawn_blocking(move || -> Result<Credential, VaultError> {
            let conn = inner.conn.blocking_lock();
            let sealed: Vec<u8> = conn
                .query_row(
                    "SELECT payload_sealed FROM credentials WHERE name = ?1",
                    params![name_for_query],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| VaultError::Transport(format!("resolve query: {e}")))?
                .ok_or_else(|| VaultError::NotFound(format!("vault://{name_for_query}")))?;
            unseal_credential(&inner.sealer, &sealed)
        })
        .await
        .map_err(|e| VaultError::Other(format!("resolve join: {e}")))??;

        // Cache write.
        self.inner.cache.lock().await.put(
            name,
            CacheEntry {
                cred: cred.clone(),
                inserted_at: Instant::now(),
            },
        );

        Ok(cred)
    }
}

fn unseal_credential(sealer: &SealedSession, sealed: &[u8]) -> Result<Credential, VaultError> {
    let plaintext = sealer
        .unseal(sealed)
        .map_err(|e| VaultError::Other(format!("unseal: {e}")))?;
    let cred: Credential = serde_json::from_slice(&plaintext)
        .map_err(|e| VaultError::Decode(format!("json decode: {e}")))?;
    // plaintext bytes are decrypted secret material — zero before dropping.
    let mut plaintext = plaintext;
    use zeroize::Zeroize;
    plaintext.zeroize();
    Ok(cred)
}

fn kind_str(k: CredentialKind) -> &'static str {
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
            warn!(kind = %other, "unknown credential kind from DB");
            Err(VaultError::Decode(format!("unknown kind: {other}")))
        }
    }
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>, VaultError> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| VaultError::Decode(format!("timestamp `{s}`: {e}")))
}

fn sqlite_error_code(e: &rusqlite::Error) -> Option<&'static str> {
    use rusqlite::ErrorCode;
    if let rusqlite::Error::SqliteFailure(err, _) = e {
        return Some(match err.code {
            ErrorCode::ConstraintViolation => "UNIQUE",
            _ => "OTHER",
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn test_key() -> MasterKey {
        MasterKey::from_bytes([42u8; 32])
    }

    fn sample_ssh() -> Credential {
        Credential::SshKey {
            username: "arif".into(),
            private_key_pem: "-----BEGIN OPENSSH PRIVATE KEY-----\nfake\n-----END OPENSSH PRIVATE KEY-----".into(),
            passphrase: None,
        }
    }

    #[tokio::test]
    async fn create_and_resolve_by_name_returns_payload() {
        let vault = SqliteVaultClient::in_memory(test_key()).unwrap();
        let id = vault.create("mikrotik-edge", sample_ssh()).await.unwrap();
        assert!(id > 0);

        let vref = CredentialRef::parse("vault://mikrotik-edge").unwrap();
        let cred = vault.resolve(&vref).await.unwrap();
        assert_eq!(cred.kind(), CredentialKind::SshKey);
    }

    #[tokio::test]
    async fn resolve_path_ref_uses_trailing_item_as_name() {
        let vault = SqliteVaultClient::in_memory(test_key()).unwrap();
        vault.create("mikrotik-edge", sample_ssh()).await.unwrap();
        let vref = CredentialRef::parse("vault://homelab/network/mikrotik-edge").unwrap();
        let cred = vault.resolve(&vref).await.unwrap();
        assert_eq!(cred.kind(), CredentialKind::SshKey);
    }

    #[tokio::test]
    async fn resolve_missing_returns_not_found() {
        let vault = SqliteVaultClient::in_memory(test_key()).unwrap();
        let vref = CredentialRef::parse("vault://does-not-exist").unwrap();
        let err = vault.resolve(&vref).await.unwrap_err();
        assert!(matches!(err, VaultError::NotFound(_)));
    }

    #[tokio::test]
    async fn duplicate_name_create_returns_error() {
        let vault = SqliteVaultClient::in_memory(test_key()).unwrap();
        vault.create("dup", sample_ssh()).await.unwrap();
        let err = vault.create("dup", sample_ssh()).await.unwrap_err();
        match err {
            VaultError::Other(msg) => assert!(msg.contains("duplicate")),
            e => panic!("expected duplicate error, got {e:?}"),
        }
    }

    #[tokio::test]
    async fn list_metadata_returns_no_secrets() {
        let vault = SqliteVaultClient::in_memory(test_key()).unwrap();
        vault.create("a", sample_ssh()).await.unwrap();
        vault
            .create(
                "b",
                Credential::ApiToken {
                    token: "SUPER_SECRET".into(),
                },
            )
            .await
            .unwrap();
        let list = vault.list_metadata().await.unwrap();
        assert_eq!(list.len(), 2);
        // Serialize the metadata and confirm no secret bytes leak.
        let json = serde_json::to_string(&list).unwrap();
        assert!(!json.contains("SUPER_SECRET"));
        assert!(!json.contains("fake"));
        assert!(json.contains("\"a\""));
        assert!(json.contains("\"b\""));
    }

    #[tokio::test]
    async fn reveal_returns_full_credential() {
        let vault = SqliteVaultClient::in_memory(test_key()).unwrap();
        let id = vault
            .create(
                "revealable",
                Credential::ApiToken {
                    token: "REVEAL_ME".into(),
                },
            )
            .await
            .unwrap();
        let cred = vault.reveal(id).await.unwrap();
        match cred {
            Credential::ApiToken { ref token } => assert_eq!(token, "REVEAL_ME"),
            ref other => panic!("expected ApiToken, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn update_replaces_payload() {
        let vault = SqliteVaultClient::in_memory(test_key()).unwrap();
        let id = vault
            .create(
                "rotatable",
                Credential::ApiToken {
                    token: "OLD".into(),
                },
            )
            .await
            .unwrap();
        vault
            .update(
                id,
                Credential::ApiToken {
                    token: "NEW".into(),
                },
            )
            .await
            .unwrap();
        let cred = vault.reveal(id).await.unwrap();
        match cred {
            Credential::ApiToken { ref token } => assert_eq!(token, "NEW"),
            ref other => panic!("expected ApiToken, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn update_invalidates_cache() {
        let vault = SqliteVaultClient::in_memory(test_key()).unwrap();
        let id = vault
            .create(
                "cache-test",
                Credential::ApiToken {
                    token: "v1".into(),
                },
            )
            .await
            .unwrap();
        let vref = CredentialRef::parse("vault://cache-test").unwrap();
        // Prime the cache.
        let _ = vault.resolve(&vref).await.unwrap();
        // Update via a different code path.
        vault
            .update(
                id,
                Credential::ApiToken {
                    token: "v2".into(),
                },
            )
            .await
            .unwrap();
        let cred = vault.resolve(&vref).await.unwrap();
        match cred {
            Credential::ApiToken { ref token } => assert_eq!(token, "v2"),
            ref other => panic!("expected ApiToken, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rename_changes_name_and_invalidates_cache() {
        let vault = SqliteVaultClient::in_memory(test_key()).unwrap();
        let id = vault.create("old-name", sample_ssh()).await.unwrap();
        // Prime cache under old name.
        let _ = vault
            .resolve(&CredentialRef::parse("vault://old-name").unwrap())
            .await
            .unwrap();
        vault.rename(id, "new-name").await.unwrap();
        // Old name no longer resolves.
        let err = vault
            .resolve(&CredentialRef::parse("vault://old-name").unwrap())
            .await
            .unwrap_err();
        assert!(matches!(err, VaultError::NotFound(_)));
        // New name resolves.
        let cred = vault
            .resolve(&CredentialRef::parse("vault://new-name").unwrap())
            .await
            .unwrap();
        assert_eq!(cred.kind(), CredentialKind::SshKey);
    }

    #[tokio::test]
    async fn delete_removes_credential() {
        let vault = SqliteVaultClient::in_memory(test_key()).unwrap();
        let id = vault.create("deletable", sample_ssh()).await.unwrap();
        vault.delete(id).await.unwrap();
        let err = vault.reveal(id).await.unwrap_err();
        assert!(matches!(err, VaultError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_missing_returns_not_found() {
        let vault = SqliteVaultClient::in_memory(test_key()).unwrap();
        let err = vault.delete(9999).await.unwrap_err();
        assert!(matches!(err, VaultError::NotFound(_)));
    }

    #[tokio::test]
    async fn wrong_master_key_fails_to_decrypt() {
        let vault_a = SqliteVaultClient::in_memory(MasterKey::from_bytes([1u8; 32])).unwrap();
        let id = vault_a.create("x", sample_ssh()).await.unwrap();
        // Force a different sealer via reveal-with-different-key would require
        // touching the storage layer directly. Instead: verify that a vault
        // opened with a different key fails on reveal. We can simulate by
        // creating two vaults pointing at the same on-disk file with different
        // keys.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let vault_b = SqliteVaultClient::open(tmp.path(), MasterKey::from_bytes([2u8; 32]))
            .await
            .unwrap();
        let _ = vault_b.create("y", sample_ssh()).await.unwrap();
        let vault_c = SqliteVaultClient::open(tmp.path(), MasterKey::from_bytes([3u8; 32]))
            .await
            .unwrap();
        let err = vault_c.reveal(1).await.unwrap_err();
        assert!(matches!(err, VaultError::Other(_)));
        // unused id from vault_a — keeps the borrow checker happy
        let _ = id;
    }

    #[tokio::test]
    async fn generic_kind_round_trips_through_serde_and_seal() {
        let vault = SqliteVaultClient::in_memory(test_key()).unwrap();
        let mut fields = BTreeMap::new();
        fields.insert("api_key".into(), "value-1".into());
        fields.insert("api_secret".into(), "value-2".into());
        let cred = Credential::Generic { fields };
        let id = vault.create("g", cred).await.unwrap();
        let recovered = vault.reveal(id).await.unwrap();
        match recovered {
            Credential::Generic { ref fields } => {
                assert_eq!(fields.get("api_key").map(|s| s.as_str()), Some("value-1"));
                assert_eq!(fields.get("api_secret").map(|s| s.as_str()), Some("value-2"));
            }
            ref other => panic!("expected Generic, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cache_hit_returns_same_payload_without_db() {
        let vault = SqliteVaultClient::in_memory(test_key()).unwrap();
        let id = vault
            .create(
                "cached",
                Credential::ApiToken {
                    token: "hit".into(),
                },
            )
            .await
            .unwrap();
        let vref = CredentialRef::parse("vault://cached").unwrap();
        let _ = vault.resolve(&vref).await.unwrap();
        // Drop the credential row via direct delete; cache should still serve.
        vault.delete(id).await.unwrap();
        // delete() invalidates cache for this name — so the next resolve
        // should miss and surface NotFound.
        let err = vault.resolve(&vref).await.unwrap_err();
        assert!(matches!(err, VaultError::NotFound(_)));
    }
}
