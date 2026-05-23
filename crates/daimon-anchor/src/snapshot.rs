//! Snapshot the current chain head for each tenant — write a manifest row to
//! `audit.anchors` and mirror a JSON file under `${DAIMON_DATA_DIR}/anchors/`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use serde::{Deserialize, Serialize};
use tokio::fs;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub tenant_id: Uuid,
    pub tenant_slug: String,
    pub as_of_ts: DateTime<Utc>,
    pub row_hash_hex: String,
    pub row_count: i64,
    pub daimon_instance_id: Uuid,
}

pub struct Snapshot {
    pub manifest: Manifest,
    pub written_to: Option<PathBuf>,
    #[allow(dead_code)]
    pub anchor_id: Uuid,
}

pub async fn snapshot_tenant(
    pool: &Pool,
    tenant_slug: &str,
    instance_id: Uuid,
    anchor_dir: Option<&PathBuf>,
) -> Result<Snapshot> {
    let client = pool.get().await.context("get pg client")?;

    let row = client
        .query_one(
            "SELECT id FROM public.tenants WHERE slug = $1",
            &[&tenant_slug],
        )
        .await
        .context("tenant lookup")?;
    let tenant_id: Uuid = row.get(0);

    let head = client
        .query_opt(
            "SELECT ts, row_hash, (SELECT COUNT(*) FROM audit.events WHERE tenant_id = $1) AS n
             FROM audit.events
             WHERE tenant_id = $1
             ORDER BY ts DESC, id DESC
             LIMIT 1",
            &[&tenant_id],
        )
        .await
        .context("chain head lookup")?;

    let (as_of_ts, row_hash, row_count) = match head {
        Some(r) => {
            let ts: DateTime<Utc> = r.get(0);
            let h: Vec<u8> = r.get(1);
            let n: i64 = r.get(2);
            (ts, h, n)
        }
        None => (Utc::now(), vec![0u8; 32], 0i64),
    };

    let anchor_row = client
        .query_one(
            "INSERT INTO audit.anchors
                (tenant_id, as_of_ts, row_hash, row_count, daimon_instance_id)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING id",
            &[
                &tenant_id,
                &as_of_ts,
                &row_hash,
                &row_count,
                &instance_id,
            ],
        )
        .await
        .context("anchor insert")?;
    let anchor_id: Uuid = anchor_row.get(0);

    let manifest = Manifest {
        tenant_id,
        tenant_slug: tenant_slug.to_string(),
        as_of_ts,
        row_hash_hex: hex::encode(&row_hash),
        row_count,
        daimon_instance_id: instance_id,
    };

    let mut written_to = None;
    if let Some(dir) = anchor_dir {
        let tenant_dir = dir.join(tenant_id.to_string());
        fs::create_dir_all(&tenant_dir)
            .await
            .with_context(|| format!("create {}", tenant_dir.display()))?;
        let filename = format!(
            "{}.json",
            as_of_ts.format("%Y-%m-%dT%H%M%S%.6fZ")
        );
        let path = tenant_dir.join(filename);
        let json = serde_json::to_vec_pretty(&manifest)?;
        fs::write(&path, json).await.context("write manifest")?;
        written_to = Some(path);
    }

    Ok(Snapshot {
        manifest,
        written_to,
        anchor_id,
    })
}

pub async fn snapshot_all(
    pool: &Pool,
    instance_id: Uuid,
    anchor_dir: Option<&PathBuf>,
) -> Result<Vec<Snapshot>> {
    let client = pool.get().await.context("get pg client")?;
    let rows = client
        .query("SELECT slug FROM public.tenants WHERE status = 'active'", &[])
        .await?;
    drop(client);

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let slug: String = row.get(0);
        let snap = snapshot_tenant(pool, &slug, instance_id, anchor_dir).await?;
        out.push(snap);
    }
    Ok(out)
}
