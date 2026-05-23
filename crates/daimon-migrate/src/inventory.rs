//! inventory.db → inventory.targets.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use rusqlite::Connection;
use uuid::Uuid;

use crate::MigrateStats;

struct Row {
    id: Uuid,
    target_ref: String,
    kind: String,
    transport: String,
    host: String,
    port: i32,
    credential_ref: String,
    labels: serde_json::Value,
    capabilities: serde_json::Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

pub async fn migrate(
    pool: &Pool,
    sqlite_path: &Path,
    tenant_id: Uuid,
    dry_run: bool,
) -> Result<MigrateStats> {
    let rows = read_sqlite(sqlite_path, tenant_id)
        .with_context(|| format!("read sqlite inventory {}", sqlite_path.display()))?;

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
                "INSERT INTO inventory.targets
                    (id, tenant_id, target_ref, kind, transport, host, port,
                     credential_ref, labels, capabilities, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                 ON CONFLICT (tenant_id, target_ref) DO UPDATE
                    SET kind = EXCLUDED.kind,
                        transport = EXCLUDED.transport,
                        host = EXCLUDED.host,
                        port = EXCLUDED.port,
                        credential_ref = EXCLUDED.credential_ref,
                        labels = EXCLUDED.labels,
                        capabilities = EXCLUDED.capabilities,
                        updated_at = EXCLUDED.updated_at",
                &[
                    &r.id,
                    &tenant_id,
                    &r.target_ref,
                    &r.kind,
                    &r.transport,
                    &r.host,
                    &r.port,
                    &r.credential_ref,
                    &r.labels,
                    &r.capabilities,
                    &r.created_at,
                    &r.updated_at,
                ],
            )
            .await
            .with_context(|| format!("insert target {}", r.target_ref))?;
        if n == 1 {
            stats.inserted += 1;
        } else {
            stats.skipped += 1;
        }
    }
    tracing::info!(target: "migrate.inventory", ?stats, "done");
    Ok(stats)
}

fn read_sqlite(path: &Path, tenant_id: Uuid) -> Result<Vec<Row>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open(path)?;
    let mut stmt = conn.prepare(
        "SELECT ref, kind, transport, host, port, credential_ref,
                labels_json, capabilities_json, created_at, updated_at
         FROM targets",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let target_ref: String = r.get(0)?;
            let kind: String = r.get(1)?;
            let transport: String = r.get(2)?;
            let host: String = r.get(3)?;
            let port: i64 = r.get(4)?;
            let cred: String = r.get(5)?;
            let labels: String = r.get(6)?;
            let caps: String = r.get(7)?;
            let created: String = r.get(8)?;
            let updated: String = r.get(9)?;
            Ok((target_ref, kind, transport, host, port, cred, labels, caps, created, updated))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::with_capacity(rows.len());
    for (target_ref, kind, transport, host, port, cred, labels, caps, created, updated) in rows {
        let id = derive_uuid(tenant_id, &target_ref);
        let labels_json: serde_json::Value = serde_json::from_str(&labels).unwrap_or_else(|_| serde_json::json!({}));
        let caps_json: serde_json::Value = serde_json::from_str(&caps).unwrap_or_else(|_| serde_json::json!([]));
        let port_i32 = port as i32;
        out.push(Row {
            id,
            target_ref,
            kind,
            transport,
            host,
            port: port_i32,
            credential_ref: cred,
            labels: labels_json,
            capabilities: caps_json,
            created_at: parse_ts(&created)?,
            updated_at: parse_ts(&updated)?,
        });
    }
    Ok(out)
}

fn derive_uuid(tenant_id: Uuid, target_ref: &str) -> Uuid {
    let input = format!("inventory/{tenant_id}/{target_ref}");
    Uuid::new_v5(&Uuid::NAMESPACE_OID, input.as_bytes())
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .with_context(|| format!("parse ts {s}"))
}
