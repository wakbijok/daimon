//! audit.db → audit.events.
//!
//! Hash chain is reconstructed by the V008 BEFORE INSERT trigger as rows
//! land in ts ASC order. Idempotency: deterministic UUID v5 from
//! (tenant_id, sqlite_id) + ON CONFLICT (id) DO NOTHING.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use rusqlite::Connection;
use uuid::Uuid;

use crate::MigrateStats;

struct Row {
    id: Uuid,
    ts: DateTime<Utc>,
    actor_id: String,
    action: String,
    target_ref: Option<String>,
    credential_ref: Option<String>,
    op_summary: Option<String>,
    result: String,
    latency_ms: Option<i32>,
    metadata: serde_json::Value,
}

pub async fn migrate(
    pool: &Pool,
    sqlite_path: &Path,
    tenant_id: Uuid,
    dry_run: bool,
) -> Result<MigrateStats> {
    let rows = read_sqlite(sqlite_path, tenant_id)
        .with_context(|| format!("read sqlite audit {}", sqlite_path.display()))?;

    let mut stats = MigrateStats {
        read: rows.len(),
        inserted: 0,
        skipped: 0,
    };

    if dry_run {
        return Ok(stats);
    }

    let client = pool.get().await?;
    for r in rows {
        let n = client
            .execute(
                "INSERT INTO audit.events
                    (id, tenant_id, ts, actor_id, action, target_ref,
                     credential_ref, op_summary, result, latency_ms, metadata)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                 ON CONFLICT (id) DO NOTHING",
                &[
                    &r.id,
                    &tenant_id,
                    &r.ts,
                    &r.actor_id,
                    &r.action,
                    &r.target_ref,
                    &r.credential_ref,
                    &r.op_summary,
                    &r.result,
                    &r.latency_ms,
                    &r.metadata,
                ],
            )
            .await
            .with_context(|| format!("insert audit event {} ({})", r.action, r.id))?;
        if n == 1 {
            stats.inserted += 1;
        } else {
            stats.skipped += 1;
        }
    }
    tracing::info!(target: "migrate.audit", ?stats, "done");
    Ok(stats)
}

fn read_sqlite(path: &Path, tenant_id: Uuid) -> Result<Vec<Row>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open(path)?;
    let mut stmt = conn.prepare(
        "SELECT id, ts, actor_id, action, target_ref, credential_ref,
                op_summary, result, latency_ms, metadata_json
         FROM audit_events
         ORDER BY ts ASC, id ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let sqlite_id: i64 = r.get(0)?;
            let ts: String = r.get(1)?;
            let actor: String = r.get(2)?;
            let action: String = r.get(3)?;
            let target: Option<String> = r.get(4)?;
            let cred: Option<String> = r.get(5)?;
            let summary: Option<String> = r.get(6)?;
            let result: String = r.get(7)?;
            let latency: Option<i64> = r.get(8)?;
            let metadata: String = r.get(9)?;
            Ok((sqlite_id, ts, actor, action, target, cred, summary, result, latency, metadata))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::with_capacity(rows.len());
    for (sqlite_id, ts, actor, action, target, cred, summary, result, latency, metadata_str) in rows {
        let id = derive_uuid(tenant_id, sqlite_id);
        let parsed_ts = DateTime::parse_from_rfc3339(&ts)
            .map(|d| d.with_timezone(&Utc))
            .with_context(|| format!("parse audit ts {ts}"))?;
        let metadata: serde_json::Value =
            serde_json::from_str(&metadata_str).unwrap_or_else(|_| serde_json::json!({}));
        let latency_i32 = latency.map(|n| n as i32);
        out.push(Row {
            id,
            ts: parsed_ts,
            actor_id: actor,
            action,
            target_ref: target,
            credential_ref: cred,
            op_summary: summary,
            result,
            latency_ms: latency_i32,
            metadata,
        });
    }
    Ok(out)
}

fn derive_uuid(tenant_id: Uuid, sqlite_id: i64) -> Uuid {
    let input = format!("audit/{tenant_id}/{sqlite_id}");
    Uuid::new_v5(&Uuid::NAMESPACE_OID, input.as_bytes())
}
