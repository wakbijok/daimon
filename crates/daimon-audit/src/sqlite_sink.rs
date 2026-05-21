//! SQLite-backed `AuditSink` impl. WAL mode + indexes + DB-level
//! UPDATE/DELETE blocking via triggers to enforce append-only semantics.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task;
use tracing::instrument;

use crate::event::{ActionKind, AuditEvent, AuditFilter, AuditResult, NewAuditEvent};
use crate::sink::{AuditError, AuditSink};

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    action TEXT NOT NULL,
    target_ref TEXT,
    credential_ref TEXT,
    op_summary TEXT,
    result TEXT NOT NULL,
    latency_ms INTEGER,
    metadata_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_events(ts);
CREATE INDEX IF NOT EXISTS idx_audit_actor ON audit_events(actor_id);
CREATE INDEX IF NOT EXISTS idx_audit_action ON audit_events(action);
CREATE INDEX IF NOT EXISTS idx_audit_target ON audit_events(target_ref);
CREATE INDEX IF NOT EXISTS idx_audit_result ON audit_events(result);

-- Append-only enforcement: SQLite triggers reject UPDATE and DELETE on the
-- audit_events table. This is a defence-in-depth measure on top of the
-- AuditSink API which only exposes append/query.
CREATE TRIGGER IF NOT EXISTS audit_events_no_update
BEFORE UPDATE ON audit_events
BEGIN
    SELECT RAISE(ABORT, 'audit_events is append-only — UPDATE blocked');
END;

CREATE TRIGGER IF NOT EXISTS audit_events_no_delete
BEFORE DELETE ON audit_events
BEGIN
    SELECT RAISE(ABORT, 'audit_events is append-only — DELETE blocked');
END;

INSERT OR IGNORE INTO schema_version (version, applied_at)
VALUES (1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
"#;

#[derive(Clone)]
pub struct SqliteAuditSink {
    inner: Arc<Inner>,
}

struct Inner {
    conn: AsyncMutex<Connection>,
}

impl SqliteAuditSink {
    pub async fn open(path: &Path) -> Result<Self, AuditError> {
        let owned_path = path.to_path_buf();
        let conn = task::spawn_blocking(move || -> Result<Connection, rusqlite::Error> {
            let conn = Connection::open(&owned_path)?;
            conn.pragma_update(None, "journal_mode", "WAL")?;
            conn.pragma_update(None, "synchronous", "NORMAL")?;
            conn.execute_batch(SCHEMA_V1)?;
            Ok(conn)
        })
        .await
        .map_err(|e| AuditError::Other(format!("open join: {e}")))?
        .map_err(|e| AuditError::Storage(format!("open: {e}")))?;
        Ok(Self::wrap(conn))
    }

    pub fn in_memory() -> Result<Self, AuditError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| AuditError::Storage(format!("in-memory open: {e}")))?;
        conn.execute_batch(SCHEMA_V1)
            .map_err(|e| AuditError::Storage(format!("schema: {e}")))?;
        Ok(Self::wrap(conn))
    }

    fn wrap(conn: Connection) -> Self {
        Self {
            inner: Arc::new(Inner {
                conn: AsyncMutex::new(conn),
            }),
        }
    }
}

#[async_trait]
impl AuditSink for SqliteAuditSink {
    #[instrument(skip(self, event), fields(actor = %event.actor_id, action = ?event.action, result = ?event.result))]
    async fn append(&self, event: NewAuditEvent) -> Result<i64, AuditError> {
        let metadata_json = serde_json::to_string(&event.metadata)
            .map_err(|e| AuditError::Serde(format!("metadata json: {e}")))?;
        let now = Utc::now().to_rfc3339();
        let action_str = event.action.as_str();
        let result_str = event.result.as_str();
        let latency = event.latency_ms.map(|m| m as i64);
        let inner = self.inner.clone();

        task::spawn_blocking(move || -> Result<i64, AuditError> {
            let conn = inner.conn.blocking_lock();
            conn.execute(
                "INSERT INTO audit_events \
                 (ts, actor_id, action, target_ref, credential_ref, op_summary, result, latency_ms, metadata_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    now,
                    event.actor_id,
                    action_str,
                    event.target_ref,
                    event.credential_ref,
                    event.op_summary,
                    result_str,
                    latency,
                    metadata_json,
                ],
            )
            .map_err(|e| AuditError::Storage(format!("insert: {e}")))?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .map_err(|e| AuditError::Other(format!("append join: {e}")))?
    }

    async fn query(
        &self,
        filter: &AuditFilter,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEvent>, AuditError> {
        let (where_sql, params_owned) = build_filter(filter);
        let limit = limit.max(1).min(10_000) as i64;
        let offset = offset as i64;
        let inner = self.inner.clone();

        task::spawn_blocking(move || -> Result<Vec<AuditEvent>, AuditError> {
            let conn = inner.conn.blocking_lock();
            let sql = format!(
                "SELECT id, ts, actor_id, action, target_ref, credential_ref, op_summary, result, latency_ms, metadata_json \
                 FROM audit_events {where_sql} ORDER BY ts DESC, id DESC LIMIT ?{} OFFSET ?{}",
                params_owned.len() + 1,
                params_owned.len() + 2,
            );
            let mut all_params: Vec<rusqlite::types::Value> = params_owned;
            all_params.push(rusqlite::types::Value::Integer(limit));
            all_params.push(rusqlite::types::Value::Integer(offset));
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| AuditError::Storage(format!("prepare query: {e}")))?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(all_params.iter()), decode_row)
                .map_err(|e| AuditError::Storage(format!("query: {e}")))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| AuditError::Storage(format!("row query: {e}")))??);
            }
            Ok(out)
        })
        .await
        .map_err(|e| AuditError::Other(format!("query join: {e}")))?
    }

    async fn count(&self, filter: &AuditFilter) -> Result<u64, AuditError> {
        let (where_sql, params_owned) = build_filter(filter);
        let inner = self.inner.clone();

        task::spawn_blocking(move || -> Result<u64, AuditError> {
            let conn = inner.conn.blocking_lock();
            let sql = format!("SELECT COUNT(*) FROM audit_events {where_sql}");
            let n: i64 = conn
                .query_row(&sql, rusqlite::params_from_iter(params_owned.iter()), |row| {
                    row.get(0)
                })
                .map_err(|e| AuditError::Storage(format!("count: {e}")))?;
            Ok(n.max(0) as u64)
        })
        .await
        .map_err(|e| AuditError::Other(format!("count join: {e}")))?
    }
}

fn build_filter(filter: &AuditFilter) -> (String, Vec<rusqlite::types::Value>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    let mut idx = 1usize;

    if let Some(actor) = &filter.actor_id {
        clauses.push(format!("actor_id = ?{idx}"));
        params.push(rusqlite::types::Value::Text(actor.clone()));
        idx += 1;
    }
    if let Some(action) = filter.action {
        clauses.push(format!("action = ?{idx}"));
        params.push(rusqlite::types::Value::Text(action.as_str().into()));
        idx += 1;
    }
    if let Some(target) = &filter.target_ref {
        clauses.push(format!("target_ref = ?{idx}"));
        params.push(rusqlite::types::Value::Text(target.clone()));
        idx += 1;
    }
    if let Some(result) = filter.result {
        clauses.push(format!("result = ?{idx}"));
        params.push(rusqlite::types::Value::Text(result.as_str().into()));
        idx += 1;
    }
    if let Some(since) = filter.since {
        clauses.push(format!("ts >= ?{idx}"));
        params.push(rusqlite::types::Value::Text(since.to_rfc3339()));
        idx += 1;
    }
    if let Some(until) = filter.until {
        clauses.push(format!("ts <= ?{idx}"));
        params.push(rusqlite::types::Value::Text(until.to_rfc3339()));
        idx += 1;
    }
    let _ = idx;

    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    (where_sql, params)
}

fn decode_row(row: &rusqlite::Row<'_>) -> Result<Result<AuditEvent, AuditError>, rusqlite::Error> {
    let id: i64 = row.get(0)?;
    let ts_str: String = row.get(1)?;
    let actor_id: String = row.get(2)?;
    let action_str: String = row.get(3)?;
    let target_ref: Option<String> = row.get(4)?;
    let credential_ref: Option<String> = row.get(5)?;
    let op_summary: Option<String> = row.get(6)?;
    let result_str: String = row.get(7)?;
    let latency_ms: Option<i64> = row.get(8)?;
    let metadata_json: String = row.get(9)?;

    Ok((|| -> Result<AuditEvent, AuditError> {
        let ts = DateTime::parse_from_rfc3339(&ts_str)
            .map_err(|e| AuditError::Storage(format!("ts parse: {e}")))?
            .with_timezone(&Utc);
        let metadata = serde_json::from_str(&metadata_json)
            .map_err(|e| AuditError::Serde(format!("metadata: {e}")))?;
        Ok(AuditEvent {
            id,
            ts,
            actor_id,
            action: ActionKind::from_str(&action_str),
            target_ref,
            credential_ref,
            op_summary,
            result: AuditResult::from_str(&result_str),
            latency_ms: latency_ms.map(|n| n as u64),
            metadata,
        })
    })())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink() -> SqliteAuditSink {
        SqliteAuditSink::in_memory().unwrap()
    }

    fn ev(actor: &str, action: ActionKind, result: AuditResult) -> NewAuditEvent {
        NewAuditEvent::new(actor, action, result)
    }

    #[tokio::test]
    async fn append_assigns_id_and_round_trips() {
        let s = sink();
        let id = s
            .append(ev("user:arif", ActionKind::VaultReveal, AuditResult::Success).with_op_summary("revealed cred id=3"))
            .await
            .unwrap();
        assert_eq!(id, 1);
        let events = s.query(&AuditFilter::default(), 10, 0).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].actor_id, "user:arif");
        assert_eq!(events[0].action, ActionKind::VaultReveal);
        assert_eq!(events[0].result, AuditResult::Success);
        assert_eq!(events[0].op_summary.as_deref(), Some("revealed cred id=3"));
    }

    #[tokio::test]
    async fn count_matches_query() {
        let s = sink();
        s.append(ev("a", ActionKind::VaultResolve, AuditResult::Success)).await.unwrap();
        s.append(ev("a", ActionKind::TransportDispatch, AuditResult::Success)).await.unwrap();
        s.append(ev("b", ActionKind::VaultResolve, AuditResult::Error)).await.unwrap();
        assert_eq!(s.count(&AuditFilter::default()).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn filter_by_actor() {
        let s = sink();
        s.append(ev("a", ActionKind::VaultResolve, AuditResult::Success)).await.unwrap();
        s.append(ev("b", ActionKind::VaultResolve, AuditResult::Success)).await.unwrap();
        s.append(ev("b", ActionKind::TransportDispatch, AuditResult::Success)).await.unwrap();
        let filter = AuditFilter {
            actor_id: Some("b".into()),
            ..Default::default()
        };
        let n = s.count(&filter).await.unwrap();
        assert_eq!(n, 2);
        let events = s.query(&filter, 10, 0).await.unwrap();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e.actor_id == "b"));
    }

    #[tokio::test]
    async fn filter_by_action_and_result() {
        let s = sink();
        s.append(ev("a", ActionKind::VaultResolve, AuditResult::Success)).await.unwrap();
        s.append(ev("a", ActionKind::VaultResolve, AuditResult::Error)).await.unwrap();
        s.append(ev("a", ActionKind::VaultReveal, AuditResult::Success)).await.unwrap();

        let filter = AuditFilter {
            action: Some(ActionKind::VaultResolve),
            result: Some(AuditResult::Error),
            ..Default::default()
        };
        let events = s.query(&filter, 10, 0).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, ActionKind::VaultResolve);
        assert_eq!(events[0].result, AuditResult::Error);
    }

    #[tokio::test]
    async fn paged_query_returns_correct_slice() {
        let s = sink();
        for i in 0..15 {
            s.append(ev(&format!("u{i}"), ActionKind::Other, AuditResult::Success))
                .await
                .unwrap();
        }
        let page1 = s.query(&AuditFilter::default(), 5, 0).await.unwrap();
        let page2 = s.query(&AuditFilter::default(), 5, 5).await.unwrap();
        assert_eq!(page1.len(), 5);
        assert_eq!(page2.len(), 5);
        // Pages should be disjoint by id.
        let ids1: Vec<_> = page1.iter().map(|e| e.id).collect();
        let ids2: Vec<_> = page2.iter().map(|e| e.id).collect();
        assert!(ids1.iter().all(|id| !ids2.contains(id)));
    }

    #[tokio::test]
    async fn order_is_newest_first() {
        let s = sink();
        let id1 = s.append(ev("a", ActionKind::Other, AuditResult::Success)).await.unwrap();
        let id2 = s.append(ev("a", ActionKind::Other, AuditResult::Success)).await.unwrap();
        let id3 = s.append(ev("a", ActionKind::Other, AuditResult::Success)).await.unwrap();
        let events = s.query(&AuditFilter::default(), 10, 0).await.unwrap();
        // Insertion order was id1 < id2 < id3, so newest-first should be [id3, id2, id1].
        assert_eq!(events[0].id, id3);
        assert_eq!(events[1].id, id2);
        assert_eq!(events[2].id, id1);
    }

    #[tokio::test]
    async fn metadata_round_trips() {
        let s = sink();
        s.append(
            ev("a", ActionKind::BrokerExecute, AuditResult::Success)
                .with_metadata("plan_id", "PL-001")
                .with_metadata("retries", "2"),
        )
        .await
        .unwrap();
        let events = s.query(&AuditFilter::default(), 10, 0).await.unwrap();
        assert_eq!(events[0].metadata.get("plan_id"), Some(&"PL-001".to_string()));
        assert_eq!(events[0].metadata.get("retries"), Some(&"2".to_string()));
    }

    #[tokio::test]
    async fn update_blocked_by_trigger() {
        let s = sink();
        let id = s
            .append(ev("a", ActionKind::Other, AuditResult::Success))
            .await
            .unwrap();
        let inner = s.inner.clone();
        let err = tokio::task::spawn_blocking(move || -> Result<(), rusqlite::Error> {
            let conn = inner.conn.blocking_lock();
            conn.execute(
                "UPDATE audit_events SET actor_id = 'evil' WHERE id = ?1",
                params![id],
            )?;
            Ok(())
        })
        .await
        .unwrap()
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("append-only") || msg.contains("UPDATE blocked"),
            "expected trigger-blocked error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn delete_blocked_by_trigger() {
        let s = sink();
        let _ = s
            .append(ev("a", ActionKind::Other, AuditResult::Success))
            .await
            .unwrap();
        let inner = s.inner.clone();
        let err = tokio::task::spawn_blocking(move || -> Result<(), rusqlite::Error> {
            let conn = inner.conn.blocking_lock();
            conn.execute("DELETE FROM audit_events", [])?;
            Ok(())
        })
        .await
        .unwrap()
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("append-only") || msg.contains("DELETE blocked"),
            "expected trigger-blocked error, got: {msg}"
        );
    }
}
