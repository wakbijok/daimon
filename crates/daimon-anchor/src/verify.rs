//! Walk the global audit chain oldest-first, recompute each row's hash from
//! the canonical payload, and report any divergence from the stored row_hash.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use uuid::Uuid;

use crate::canonical::{AuditRow, GENESIS_PREV_HASH, canonicalize, compute_row_hash};

#[derive(Debug, Clone)]
pub struct ChainBreak {
    pub event_id: Uuid,
    pub ts: DateTime<Utc>,
    pub expected_hex: String,
    pub stored_hex: String,
}

#[derive(Debug)]
pub struct VerifyReport {
    pub rows_checked: usize,
    pub breaks: Vec<ChainBreak>,
}

pub async fn verify(pool: &Pool) -> Result<VerifyReport> {
    let client = pool.get().await.context("get pg client")?;

    let rows = client
        .query(
            "SELECT id, ts, actor_id, action, target_ref, credential_ref,
                    op_summary, result, latency_ms, metadata::TEXT, prev_hash, row_hash
             FROM audit.events
             ORDER BY ts ASC, id ASC",
            &[],
        )
        .await
        .context("audit scan")?;

    let mut expected_prev: [u8; 32] = GENESIS_PREV_HASH;
    let mut breaks = Vec::new();
    let rows_checked = rows.len();

    for r in rows {
        let event_id: Uuid = r.get(0);
        let ts: DateTime<Utc> = r.get(1);
        let actor_id: String = r.get(2);
        let action: String = r.get(3);
        let target_ref: Option<String> = r.get(4);
        let credential_ref: Option<String> = r.get(5);
        let op_summary: Option<String> = r.get(6);
        let result: String = r.get(7);
        let latency_ms: Option<i32> = r.get(8);
        let metadata_text: String = r.get(9);
        let _prev_hash: Vec<u8> = r.get(10);
        let stored_hash: Vec<u8> = r.get(11);

        let canonical = canonicalize(&AuditRow {
            ts,
            actor_id: &actor_id,
            action: &action,
            target_ref: target_ref.as_deref(),
            credential_ref: credential_ref.as_deref(),
            op_summary: op_summary.as_deref(),
            result: &result,
            latency_ms: latency_ms.map(|n| n as i64),
            metadata_json: &metadata_text,
        });
        let computed = compute_row_hash(&canonical, &expected_prev);

        if computed.as_slice() != stored_hash.as_slice() {
            breaks.push(ChainBreak {
                event_id,
                ts,
                expected_hex: hex::encode(computed),
                stored_hex: hex::encode(&stored_hash),
            });
        }

        let mut next: [u8; 32] = [0u8; 32];
        next.copy_from_slice(&stored_hash);
        expected_prev = next;
    }

    Ok(VerifyReport {
        rows_checked,
        breaks,
    })
}
