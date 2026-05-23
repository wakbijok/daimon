//! Postgres-backed `AuditSink` impl (Phase 2c D3b).
//!
//! Append + paged query against `audit.events` in the relational tier.
//! The V008 BEFORE INSERT trigger computes prev_hash + row_hash per-tenant.
//! V005 triggers block UPDATE/DELETE at DB level (defence in depth on top
//! of the AuditSink API).
//!
//! Multi-tenancy: each instance is scoped to a single tenant_id. D6 wires
//! tenant routing at the AppState level.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use daimon_db::Pool;
use std::collections::BTreeMap;
use tracing::instrument;
use uuid::Uuid;

use crate::event::{ActionKind, AuditEvent, AuditFilter, AuditResult, NewAuditEvent};
use crate::sink::{AuditError, AuditSink};

#[derive(Clone)]
pub struct PostgresAuditSink {
    pool: Pool,
    tenant_id: Uuid,
}

impl PostgresAuditSink {
    pub fn new(pool: Pool, tenant_id: Uuid) -> Self {
        Self { pool, tenant_id }
    }
}

#[async_trait]
impl AuditSink for PostgresAuditSink {
    #[instrument(skip(self, event), level = "debug")]
    async fn append(&self, event: NewAuditEvent) -> Result<Uuid, AuditError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| AuditError::Storage(format!("pool: {e}")))?;
        let metadata: serde_json::Value = serde_json::to_value(&event.metadata)
            .map_err(|e| AuditError::Serde(format!("metadata: {e}")))?;
        let latency_i32 = event.latency_ms.map(|n| n as i32);
        let action_str = event.action.as_str();
        let result_str = event.result.as_str();
        let row = client
            .query_one(
                "INSERT INTO audit.events
                    (tenant_id, actor_id, action, target_ref, credential_ref,
                     op_summary, result, latency_ms, metadata)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 RETURNING id",
                &[
                    &self.tenant_id,
                    &event.actor_id,
                    &action_str,
                    &event.target_ref,
                    &event.credential_ref,
                    &event.op_summary,
                    &result_str,
                    &latency_i32,
                    &metadata,
                ],
            )
            .await
            .map_err(|e| AuditError::Storage(format!("append: {e}")))?;
        Ok(row.get(0))
    }

    async fn query(
        &self,
        filter: &AuditFilter,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEvent>, AuditError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| AuditError::Storage(format!("pool: {e}")))?;
        let (sql, params) = build_query(filter, &self.tenant_id, Some((limit, offset)));
        let pgs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| &**p as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();
        let rows = client
            .query(sql.as_str(), pgs.as_slice())
            .await
            .map_err(|e| AuditError::Storage(format!("query: {e}")))?;
        rows.into_iter().map(row_to_event).collect()
    }

    async fn count(&self, filter: &AuditFilter) -> Result<u64, AuditError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| AuditError::Storage(format!("pool: {e}")))?;
        let (sql, params) = build_count(filter, &self.tenant_id);
        let pgs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| &**p as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();
        let row = client
            .query_one(sql.as_str(), pgs.as_slice())
            .await
            .map_err(|e| AuditError::Storage(format!("count: {e}")))?;
        let n: i64 = row.get(0);
        Ok(n as u64)
    }
}

// Parameter container — boxed so SQL builder can mix types in one Vec.
type Param = Box<dyn tokio_postgres::types::ToSql + Sync + Send>;

fn build_query(
    filter: &AuditFilter,
    tenant_id: &Uuid,
    paging: Option<(u32, u32)>,
) -> (String, Vec<Param>) {
    let mut sql = String::from(
        "SELECT id, ts, actor_id, action, target_ref, credential_ref,
                op_summary, result, latency_ms, metadata
         FROM audit.events
         WHERE tenant_id = $1",
    );
    let mut params: Vec<Param> = vec![Box::new(*tenant_id)];
    push_filter_clauses(&mut sql, &mut params, filter);
    sql.push_str(" ORDER BY ts DESC, id DESC");
    if let Some((limit, offset)) = paging {
        sql.push_str(&format!(" LIMIT ${} OFFSET ${}", params.len() + 1, params.len() + 2));
        params.push(Box::new(limit as i64));
        params.push(Box::new(offset as i64));
    }
    (sql, params)
}

fn build_count(filter: &AuditFilter, tenant_id: &Uuid) -> (String, Vec<Param>) {
    let mut sql = String::from("SELECT COUNT(*) FROM audit.events WHERE tenant_id = $1");
    let mut params: Vec<Param> = vec![Box::new(*tenant_id)];
    push_filter_clauses(&mut sql, &mut params, filter);
    (sql, params)
}

fn push_filter_clauses(sql: &mut String, params: &mut Vec<Param>, filter: &AuditFilter) {
    if let Some(actor) = &filter.actor_id {
        sql.push_str(&format!(" AND actor_id = ${}", params.len() + 1));
        params.push(Box::new(actor.clone()));
    }
    if let Some(action) = &filter.action {
        sql.push_str(&format!(" AND action = ${}", params.len() + 1));
        params.push(Box::new(action.as_str().to_string()));
    }
    if let Some(t) = &filter.target_ref {
        sql.push_str(&format!(" AND target_ref ILIKE ${}", params.len() + 1));
        params.push(Box::new(format!("%{t}%")));
    }
    if let Some(res) = &filter.result {
        sql.push_str(&format!(" AND result = ${}", params.len() + 1));
        params.push(Box::new(res.as_str().to_string()));
    }
    if let Some(since) = filter.since {
        sql.push_str(&format!(" AND ts >= ${}", params.len() + 1));
        params.push(Box::new(since));
    }
    if let Some(until) = filter.until {
        sql.push_str(&format!(" AND ts < ${}", params.len() + 1));
        params.push(Box::new(until));
    }
}

fn row_to_event(row: tokio_postgres::Row) -> Result<AuditEvent, AuditError> {
    let id: Uuid = row.get(0);
    let ts: DateTime<Utc> = row.get(1);
    let actor_id: String = row.get(2);
    let action_str: String = row.get(3);
    let target_ref: Option<String> = row.get(4);
    let credential_ref: Option<String> = row.get(5);
    let op_summary: Option<String> = row.get(6);
    let result_str: String = row.get(7);
    let latency_ms_opt: Option<i32> = row.get(8);
    let metadata_val: serde_json::Value = row.get(9);
    let metadata: BTreeMap<String, String> =
        serde_json::from_value(metadata_val).unwrap_or_default();
    Ok(AuditEvent {
        id,
        ts,
        actor_id,
        action: ActionKind::from_str(&action_str),
        target_ref,
        credential_ref,
        op_summary,
        result: AuditResult::from_str(&result_str),
        latency_ms: latency_ms_opt.map(|n| n as u64),
        metadata,
    })
}
