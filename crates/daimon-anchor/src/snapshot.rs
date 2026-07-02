//! Snapshot the current chain head — write a manifest row to `audit.anchors`
//! and mirror a JSON file under `${DAIMON_DATA_DIR}/anchors/`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use serde::{Deserialize, Serialize};
use tokio::fs;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
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

pub async fn snapshot(
    pool: &Pool,
    instance_id: Uuid,
    anchor_dir: Option<&PathBuf>,
) -> Result<Snapshot> {
    let client = pool.get().await.context("get pg client")?;

    let head = client
        .query_opt(
            "SELECT ts, row_hash, (SELECT COUNT(*) FROM audit.events) AS n
             FROM audit.events
             ORDER BY ts DESC, id DESC
             LIMIT 1",
            &[],
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
                (as_of_ts, row_hash, row_count, daimon_instance_id)
             VALUES ($1, $2, $3, $4)
             RETURNING id",
            &[&as_of_ts, &row_hash, &row_count, &instance_id],
        )
        .await
        .context("anchor insert")?;
    let anchor_id: Uuid = anchor_row.get(0);

    let manifest = Manifest {
        as_of_ts,
        row_hash_hex: hex::encode(&row_hash),
        row_count,
        daimon_instance_id: instance_id,
    };

    let mut written_to = None;
    if let Some(dir) = anchor_dir {
        fs::create_dir_all(dir)
            .await
            .with_context(|| format!("create {}", dir.display()))?;
        let filename = format!("{}.json", as_of_ts.format("%Y-%m-%dT%H%M%S%.6fZ"));
        let path = dir.join(filename);
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
