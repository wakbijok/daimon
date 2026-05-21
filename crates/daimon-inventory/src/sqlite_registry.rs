//! SQLite-backed `Inventory` impl for production deployments (D20, Phase 2b).
//!
//! Stores managed targets in a SQLite database with WAL mode enabled for
//! concurrent reads + a single writer. Schema is versioned via a
//! `schema_version` table so future migrations have a hook.
//!
//! Concurrency: rusqlite is sync, so each operation wraps in
//! `tokio::task::spawn_blocking`. A single `Connection` lives behind
//! `Arc<Mutex<Connection>>` — SQLite serialises writes internally; we don't
//! need a multi-connection pool for inventory-scale traffic. WAL mode lets
//! reads run concurrently with writes when needed.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task;
use tracing::{debug, instrument};

use crate::refspec::TargetRef;
use crate::registry::{Inventory, InventoryError};
use crate::target::{ManagedTarget, TargetKind, TargetMetadata, TransportKind};

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS targets (
    ref TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    transport TEXT NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL,
    credential_ref TEXT NOT NULL,
    labels_json TEXT NOT NULL DEFAULT '{}',
    capabilities_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_targets_kind ON targets(kind);
CREATE INDEX IF NOT EXISTS idx_targets_transport ON targets(transport);

INSERT OR IGNORE INTO schema_version (version, applied_at)
VALUES (1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
"#;

/// SQLite-backed inventory.
#[derive(Clone)]
pub struct SqliteRegistry {
    inner: Arc<Inner>,
}

struct Inner {
    conn: AsyncMutex<Connection>,
}

impl SqliteRegistry {
    /// Open or create the inventory DB at the given path. Enables WAL mode
    /// and foreign keys.
    pub async fn open(path: &Path) -> Result<Self, InventoryError> {
        let owned_path = path.to_path_buf();
        let conn = task::spawn_blocking(move || -> Result<Connection, rusqlite::Error> {
            let conn = Connection::open(&owned_path)?;
            conn.pragma_update(None, "journal_mode", "WAL")?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            conn.pragma_update(None, "synchronous", "NORMAL")?;
            conn.execute_batch(SCHEMA_V1)?;
            Ok(conn)
        })
        .await
        .map_err(|e| InventoryError::Other(format!("open join: {e}")))?
        .map_err(|e| InventoryError::Other(format!("open: {e}")))?;
        Ok(Self::wrap(conn))
    }

    /// In-memory inventory for tests.
    pub fn in_memory() -> Result<Self, InventoryError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| InventoryError::Other(format!("in-memory open: {e}")))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| InventoryError::Other(format!("pragma fk: {e}")))?;
        conn.execute_batch(SCHEMA_V1)
            .map_err(|e| InventoryError::Other(format!("schema: {e}")))?;
        Ok(Self::wrap(conn))
    }

    fn wrap(conn: Connection) -> Self {
        Self {
            inner: Arc::new(Inner {
                conn: AsyncMutex::new(conn),
            }),
        }
    }

    /// Count targets — primarily for tests / admin counters.
    pub async fn count(&self) -> Result<u64, InventoryError> {
        let inner = self.inner.clone();
        task::spawn_blocking(move || -> Result<u64, InventoryError> {
            let conn = inner.conn.blocking_lock();
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM targets", [], |row| row.get(0))
                .map_err(|e| InventoryError::Other(format!("count: {e}")))?;
            Ok(n as u64)
        })
        .await
        .map_err(|e| InventoryError::Other(format!("count join: {e}")))?
    }
}

#[async_trait]
impl Inventory for SqliteRegistry {
    #[instrument(skip(self), fields(ref = %r#ref))]
    async fn get_metadata(&self, r#ref: &TargetRef) -> Result<TargetMetadata, InventoryError> {
        let mt = self.get_managed(r#ref).await?;
        Ok(mt.metadata())
    }

    #[instrument(skip(self), fields(ref = %r#ref))]
    async fn get_managed(&self, r#ref: &TargetRef) -> Result<ManagedTarget, InventoryError> {
        let key = r#ref.to_string();
        let inner = self.inner.clone();
        task::spawn_blocking(move || -> Result<ManagedTarget, InventoryError> {
            let conn = inner.conn.blocking_lock();
            row_to_managed(
                conn.query_row(
                    "SELECT ref, kind, transport, host, port, credential_ref, labels_json, capabilities_json \
                     FROM targets WHERE ref = ?1",
                    params![key],
                    decode_row,
                )
                .optional()
                .map_err(|e| InventoryError::Other(format!("get: {e}")))?
                .ok_or_else(|| InventoryError::NotFound(key.clone()))?,
            )
        })
        .await
        .map_err(|e| InventoryError::Other(format!("get join: {e}")))?
    }

    async fn list(&self, kind_filter: Option<TargetKind>) -> Vec<TargetMetadata> {
        let inner = self.inner.clone();
        let kind_str = kind_filter.map(kind_to_str);
        let result = task::spawn_blocking(move || -> Result<Vec<ManagedTarget>, InventoryError> {
            let conn = inner.conn.blocking_lock();
            let (sql, params): (&str, Vec<rusqlite::types::Value>) = match kind_str {
                Some(k) => (
                    "SELECT ref, kind, transport, host, port, credential_ref, labels_json, capabilities_json \
                     FROM targets WHERE kind = ?1 ORDER BY ref",
                    vec![rusqlite::types::Value::Text(k.into())],
                ),
                None => (
                    "SELECT ref, kind, transport, host, port, credential_ref, labels_json, capabilities_json \
                     FROM targets ORDER BY ref",
                    vec![],
                ),
            };
            let mut stmt = conn
                .prepare(sql)
                .map_err(|e| InventoryError::Other(format!("prepare list: {e}")))?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(params.iter()), decode_row)
                .map_err(|e| InventoryError::Other(format!("query list: {e}")))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row_to_managed(
                    row.map_err(|e| InventoryError::Other(format!("row list: {e}")))?,
                )?);
            }
            Ok(out)
        })
        .await;
        match result {
            Ok(Ok(targets)) => targets.into_iter().map(|t| t.metadata()).collect(),
            Ok(Err(e)) => {
                tracing::error!(error = %e, "inventory list failed");
                Vec::new()
            }
            Err(e) => {
                tracing::error!(error = %e, "inventory list join failed");
                Vec::new()
            }
        }
    }

    #[instrument(skip(self, target), fields(ref = %target.r#ref))]
    async fn upsert(&self, target: ManagedTarget) -> Result<(), InventoryError> {
        let labels_json = serde_json::to_string(&target.labels)
            .map_err(|e| InventoryError::Other(format!("labels json: {e}")))?;
        let capabilities_json = serde_json::to_string(&target.capabilities)
            .map_err(|e| InventoryError::Other(format!("capabilities json: {e}")))?;
        let now = Utc::now().to_rfc3339();
        let ref_str = target.r#ref.to_string();
        let inner = self.inner.clone();

        task::spawn_blocking(move || -> Result<(), InventoryError> {
            let conn = inner.conn.blocking_lock();
            conn.execute(
                "INSERT INTO targets (ref, kind, transport, host, port, credential_ref, labels_json, capabilities_json, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9) \
                 ON CONFLICT(ref) DO UPDATE SET \
                    kind = excluded.kind, \
                    transport = excluded.transport, \
                    host = excluded.host, \
                    port = excluded.port, \
                    credential_ref = excluded.credential_ref, \
                    labels_json = excluded.labels_json, \
                    capabilities_json = excluded.capabilities_json, \
                    updated_at = excluded.updated_at",
                params![
                    ref_str,
                    kind_to_str(target.kind),
                    transport_to_str(target.transport),
                    target.host,
                    target.port as i64,
                    target.credential_ref,
                    labels_json,
                    capabilities_json,
                    now,
                ],
            )
            .map_err(|e| InventoryError::Other(format!("upsert: {e}")))?;
            Ok(())
        })
        .await
        .map_err(|e| InventoryError::Other(format!("upsert join: {e}")))??;

        debug!("inventory upsert ok");
        Ok(())
    }

    #[instrument(skip(self), fields(ref = %r#ref))]
    async fn remove(&self, r#ref: &TargetRef) -> Result<(), InventoryError> {
        let key = r#ref.to_string();
        let inner = self.inner.clone();
        task::spawn_blocking(move || -> Result<(), InventoryError> {
            let conn = inner.conn.blocking_lock();
            let n = conn
                .execute("DELETE FROM targets WHERE ref = ?1", params![key])
                .map_err(|e| InventoryError::Other(format!("delete: {e}")))?;
            if n == 0 {
                return Err(InventoryError::NotFound(key));
            }
            Ok(())
        })
        .await
        .map_err(|e| InventoryError::Other(format!("remove join: {e}")))?
    }
}

fn kind_to_str(k: TargetKind) -> &'static str {
    match k {
        TargetKind::Platform => "platform",
        TargetKind::Network => "network",
        TargetKind::Host => "host",
        TargetKind::App => "app",
    }
}

fn str_to_kind(s: &str) -> Result<TargetKind, InventoryError> {
    match s {
        "platform" => Ok(TargetKind::Platform),
        "network" => Ok(TargetKind::Network),
        "host" => Ok(TargetKind::Host),
        "app" => Ok(TargetKind::App),
        other => Err(InventoryError::Other(format!("unknown kind: {other}"))),
    }
}

fn transport_to_str(t: TransportKind) -> &'static str {
    match t {
        TransportKind::Ssh => "ssh",
        TransportKind::Rest => "rest",
        TransportKind::Snmp => "snmp",
        TransportKind::Grpc => "grpc",
    }
}

fn str_to_transport(s: &str) -> Result<TransportKind, InventoryError> {
    match s {
        "ssh" => Ok(TransportKind::Ssh),
        "rest" => Ok(TransportKind::Rest),
        "snmp" => Ok(TransportKind::Snmp),
        "grpc" => Ok(TransportKind::Grpc),
        other => Err(InventoryError::Other(format!("unknown transport: {other}"))),
    }
}

struct RawRow {
    r#ref: String,
    kind: String,
    transport: String,
    host: String,
    port: i64,
    credential_ref: String,
    labels_json: String,
    capabilities_json: String,
}

fn decode_row(row: &rusqlite::Row<'_>) -> Result<RawRow, rusqlite::Error> {
    Ok(RawRow {
        r#ref: row.get(0)?,
        kind: row.get(1)?,
        transport: row.get(2)?,
        host: row.get(3)?,
        port: row.get(4)?,
        credential_ref: row.get(5)?,
        labels_json: row.get(6)?,
        capabilities_json: row.get(7)?,
    })
}

fn row_to_managed(row: RawRow) -> Result<ManagedTarget, InventoryError> {
    let r#ref = TargetRef::parse(&row.r#ref)
        .map_err(|e| InventoryError::Other(format!("ref parse `{}`: {e}", row.r#ref)))?;
    let kind = str_to_kind(&row.kind)?;
    let transport = str_to_transport(&row.transport)?;
    let labels = serde_json::from_str(&row.labels_json)
        .map_err(|e| InventoryError::Other(format!("labels json: {e}")))?;
    let capabilities = serde_json::from_str(&row.capabilities_json)
        .map_err(|e| InventoryError::Other(format!("capabilities json: {e}")))?;
    Ok(ManagedTarget {
        r#ref,
        kind,
        transport,
        host: row.host,
        port: row.port as u16,
        credential_ref: row.credential_ref,
        labels,
        capabilities,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn sample(name: &str, kind: TargetKind) -> ManagedTarget {
        ManagedTarget {
            r#ref: TargetRef::parse(&format!("target://{name}")).unwrap(),
            kind,
            transport: TransportKind::Ssh,
            host: format!("{name}.local"),
            port: 22,
            credential_ref: format!("vault://{name}"),
            labels: BTreeMap::new(),
            capabilities: vec!["network.firewall.address_list.add".into()],
        }
    }

    #[tokio::test]
    async fn upsert_and_get_managed_round_trip() {
        let reg = SqliteRegistry::in_memory().unwrap();
        let t = sample("mikrotik-edge", TargetKind::Network);
        reg.upsert(t.clone()).await.unwrap();
        let got = reg.get_managed(&t.r#ref).await.unwrap();
        assert_eq!(got, t);
    }

    #[tokio::test]
    async fn metadata_view_strips_credential_ref() {
        let reg = SqliteRegistry::in_memory().unwrap();
        let t = sample("mikrotik-edge", TargetKind::Network);
        reg.upsert(t.clone()).await.unwrap();
        let md = reg.get_metadata(&t.r#ref).await.unwrap();
        let json = serde_json::to_string(&md).unwrap();
        assert!(!json.contains("vault://"));
        assert!(!json.contains("credential_ref"));
    }

    #[tokio::test]
    async fn upsert_replaces_existing() {
        let reg = SqliteRegistry::in_memory().unwrap();
        let mut t = sample("mikrotik-edge", TargetKind::Network);
        reg.upsert(t.clone()).await.unwrap();
        t.host = "10.100.10.1".into();
        reg.upsert(t.clone()).await.unwrap();
        let got = reg.get_managed(&t.r#ref).await.unwrap();
        assert_eq!(got.host, "10.100.10.1");
        assert_eq!(reg.count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn list_filters_by_kind() {
        let reg = SqliteRegistry::in_memory().unwrap();
        reg.upsert(sample("mikrotik-edge", TargetKind::Network))
            .await
            .unwrap();
        reg.upsert(sample("nargothrond", TargetKind::Host))
            .await
            .unwrap();
        reg.upsert(sample("nargothrond-pve", TargetKind::Platform))
            .await
            .unwrap();
        let nets = reg.list(Some(TargetKind::Network)).await;
        assert_eq!(nets.len(), 1);
        let hosts = reg.list(Some(TargetKind::Host)).await;
        assert_eq!(hosts.len(), 1);
        let all = reg.list(None).await;
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn list_returns_metadata_only_not_credential_ref() {
        let reg = SqliteRegistry::in_memory().unwrap();
        reg.upsert(sample("a", TargetKind::Network)).await.unwrap();
        reg.upsert(sample("b", TargetKind::Host)).await.unwrap();
        let list = reg.list(None).await;
        let json = serde_json::to_string(&list).unwrap();
        assert!(!json.contains("vault://"));
        assert!(!json.contains("credential_ref"));
    }

    #[tokio::test]
    async fn remove_missing_returns_not_found() {
        let reg = SqliteRegistry::in_memory().unwrap();
        let r = TargetRef::parse("target://nonexistent").unwrap();
        let err = reg.remove(&r).await.unwrap_err();
        assert!(matches!(err, InventoryError::NotFound(_)));
    }

    #[tokio::test]
    async fn remove_then_get_returns_not_found() {
        let reg = SqliteRegistry::in_memory().unwrap();
        let t = sample("removable", TargetKind::Host);
        reg.upsert(t.clone()).await.unwrap();
        reg.remove(&t.r#ref).await.unwrap();
        let err = reg.get_managed(&t.r#ref).await.unwrap_err();
        assert!(matches!(err, InventoryError::NotFound(_)));
    }

    #[tokio::test]
    async fn labels_persist_through_round_trip() {
        let reg = SqliteRegistry::in_memory().unwrap();
        let mut t = sample("labeled", TargetKind::Host);
        t.labels.insert("env".into(), "prod".into());
        t.labels.insert("rack".into(), "R1".into());
        reg.upsert(t.clone()).await.unwrap();
        let got = reg.get_managed(&t.r#ref).await.unwrap();
        assert_eq!(got.labels, t.labels);
    }

    #[tokio::test]
    async fn file_backed_persists_across_reopen() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp); // we just want the path
        {
            let reg = SqliteRegistry::open(&path).await.unwrap();
            reg.upsert(sample("persistent", TargetKind::Host))
                .await
                .unwrap();
        }
        let reg2 = SqliteRegistry::open(&path).await.unwrap();
        let got = reg2
            .get_managed(&TargetRef::parse("target://persistent").unwrap())
            .await
            .unwrap();
        assert_eq!(got.host, "persistent.local");
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(format!("{}-shm", path.display())).ok();
        std::fs::remove_file(format!("{}-wal", path.display())).ok();
    }
}
