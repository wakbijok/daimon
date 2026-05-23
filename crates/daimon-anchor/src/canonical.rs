//! Rust mirror of the `audit.compute_row_hash` trigger in V008.
//!
//! Keep these two implementations behaviourally identical. Any change to the
//! canonical payload here MUST be paired with a matching change in
//! `migrations/V008__audit_hash_chain.sql`, then verified by inserting a row
//! via SQL and checking that `compute_row_hash` (here) reproduces the trigger
//! output bit-for-bit.

use chrono::{DateTime, SecondsFormat, Utc};
use sha2::{Digest, Sha256};

pub const GENESIS_PREV_HASH: [u8; 32] = [0u8; 32];

pub struct AuditRow<'a> {
    pub ts: DateTime<Utc>,
    pub actor_id: &'a str,
    pub action: &'a str,
    pub target_ref: Option<&'a str>,
    pub credential_ref: Option<&'a str>,
    pub op_summary: Option<&'a str>,
    pub result: &'a str,
    pub latency_ms: Option<i64>,
    pub metadata_json: &'a str,
}

pub fn canonicalize(row: &AuditRow<'_>) -> Vec<u8> {
    let ts = row.ts.to_rfc3339_opts(SecondsFormat::Micros, true);
    let latency = row.latency_ms.map(|n| n.to_string()).unwrap_or_default();
    let parts = [
        ts.as_str(),
        row.actor_id,
        row.action,
        row.target_ref.unwrap_or_default(),
        row.credential_ref.unwrap_or_default(),
        row.op_summary.unwrap_or_default(),
        row.result,
        latency.as_str(),
        if row.metadata_json.is_empty() {
            "{}"
        } else {
            row.metadata_json
        },
    ];
    parts.join("|").into_bytes()
}

pub fn compute_row_hash(canonical: &[u8], prev_hash: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(canonical);
    hasher.update(prev_hash);
    hasher.finalize().into()
}
