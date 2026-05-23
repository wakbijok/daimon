//! vault.db → vault.credentials.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use rusqlite::Connection;
use uuid::Uuid;

use crate::MigrateStats;

struct Row {
    id: Uuid,
    name: String,
    kind: String,
    payload_sealed: Vec<u8>,
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
        .with_context(|| format!("read sqlite vault {}", sqlite_path.display()))?;

    let mut stats = MigrateStats {
        read: rows.len(),
        inserted: 0,
        skipped: 0,
    };

    if dry_run {
        tracing::info!(target: "migrate.vault", count = rows.len(), "dry-run skip");
        return Ok(stats);
    }

    let client = pool.get().await?;
    for r in rows {
        let n = client
            .execute(
                "INSERT INTO vault.credentials
                    (id, tenant_id, name, kind, payload_sealed, encryption_version, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, 1, $6, $7)
                 ON CONFLICT (tenant_id, name) DO UPDATE
                    SET kind = EXCLUDED.kind,
                        payload_sealed = EXCLUDED.payload_sealed,
                        updated_at = EXCLUDED.updated_at",
                &[
                    &r.id,
                    &tenant_id,
                    &r.name,
                    &r.kind,
                    &r.payload_sealed,
                    &r.created_at,
                    &r.updated_at,
                ],
            )
            .await
            .with_context(|| format!("insert credential {}", r.name))?;
        if n == 1 {
            stats.inserted += 1;
        } else {
            stats.skipped += 1;
        }
    }
    tracing::info!(target: "migrate.vault", ?stats, "done");
    Ok(stats)
}

fn read_sqlite(path: &Path, tenant_id: Uuid) -> Result<Vec<Row>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open(path)?;
    let mut stmt = conn.prepare(
        "SELECT id, name, kind, payload_sealed, created_at, updated_at FROM credentials",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let sqlite_id: i64 = r.get(0)?;
            let name: String = r.get(1)?;
            let kind: String = r.get(2)?;
            let payload: Vec<u8> = r.get(3)?;
            let created: String = r.get(4)?;
            let updated: String = r.get(5)?;
            Ok((sqlite_id, name, kind, payload, created, updated))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::with_capacity(rows.len());
    for (sqlite_id, name, kind, payload, created, updated) in rows {
        let id = derive_uuid(tenant_id, sqlite_id);
        let created_at = parse_ts(&created)?;
        let updated_at = parse_ts(&updated)?;
        out.push(Row {
            id,
            name,
            kind,
            payload_sealed: payload,
            created_at,
            updated_at,
        });
    }
    Ok(out)
}

fn derive_uuid(tenant_id: Uuid, sqlite_id: i64) -> Uuid {
    let input = format!("vault/{tenant_id}/{sqlite_id}");
    Uuid::new_v5(&Uuid::NAMESPACE_OID, input.as_bytes())
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .with_context(|| format!("parse ts {s}"))
}
