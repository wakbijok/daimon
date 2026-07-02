//! `/admin/memory` — server-fns backing the memory admin UI.
//!
//! Two operations, both routed through the `MemoryService` trait held in
//! `AppState` (P3 — long-term memory is the dmem SIDECAR, not an embedded
//! Qdrant store):
//! - `admin_memory_ingest` — ingest text into long-term memory.
//! - `admin_memory_search` — retrieve scored hits for a query.
//!
//! Both gated by `require_admin()` (D24). Every call still emits a memory-tier
//! audit event via the broker (`MemoryIngest` / `MemoryRetrieve`).

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestRequest {
    pub source_id: String,
    pub source_kind: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestResult {
    pub source_id: String,
    pub chunks: usize,
    pub collection: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub top_k: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub id: u64,
    pub score: f32,
    pub source_id: String,
    pub source_kind: String,
    pub chunk_index: u64,
    pub text: String,
}

#[server]
pub async fn admin_memory_ingest(req: IngestRequest) -> Result<IngestResult, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;
    use daimon_broker::ActionKind;
    use daimon_memory::IngestDoc;
    use std::time::Instant;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();

    let doc = IngestDoc {
        source_id: req.source_id.clone(),
        source_kind: req.source_kind.clone(),
        content: req.content,
    };

    let target = format!("memory://{}", doc.source_id);
    let start = Instant::now();
    let result = state.memory.ingest(doc).await;
    let latency_ms = start.elapsed().as_millis() as u64;

    let (success, summary, chunks_meta) = match &result {
        Ok(s) => (true, format!("ingested {} chunks", s.chunks), s.chunks.to_string()),
        Err(e) => (false, format!("ingest failed: {}", e), "0".to_string()),
    };

    let _ = state
        .broker
        .audit_memory_op(
            &claims.sub,
            ActionKind::MemoryIngest,
            Some(&target),
            Some(&summary),
            latency_ms,
            success,
            vec![
                ("source_id".to_string(), req.source_id.clone()),
                ("source_kind".to_string(), req.source_kind.clone()),
                ("chunks".to_string(), chunks_meta),
            ],
        )
        .await;

    let stats = result.map_err(|e| ServerFnError::new(format!("ingest: {}", e)))?;
    Ok(IngestResult {
        source_id: stats.source_id,
        chunks: stats.chunks,
        collection: stats.collection,
    })
}

#[server]
pub async fn admin_memory_search(req: SearchRequest) -> Result<Vec<SearchHit>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;
    use daimon_broker::ActionKind;
    use daimon_memory::RetrieveQuery;
    use std::time::Instant;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();

    let target = "memory://_search".to_string();
    let start = Instant::now();
    let hits_res = state
        .memory
        .retrieve(&RetrieveQuery {
            query: req.query.clone(),
            top_k: req.top_k,
        })
        .await;
    let latency_ms = start.elapsed().as_millis() as u64;

    let (success, summary, returned_count) = match &hits_res {
        Ok(h) => (true, format!("returned {} hits", h.len()), h.len().to_string()),
        Err(e) => (false, format!("retrieve failed: {}", e), "0".to_string()),
    };

    let _ = state
        .broker
        .audit_memory_op(
            &claims.sub,
            ActionKind::MemoryRetrieve,
            Some(&target),
            Some(&summary),
            latency_ms,
            success,
            vec![
                ("query_hash".to_string(), format!("{:x}", hash_query(&req.query))),
                ("top_k".to_string(), req.top_k.to_string()),
                ("returned".to_string(), returned_count),
            ],
        )
        .await;

    let hits = hits_res.map_err(|e| ServerFnError::new(format!("retrieve: {}", e)))?;

    // Map RetrievedChunk → SearchHit. The sidecar has no numeric chunk id or
    // chunk index (records are whole), so `id`/`chunk_index` are 0; `source_id`
    // + `source_kind` carry the record's namespace + kind, `text` the body, and
    // `score` the rank-derived synthesized score.
    let out: Vec<SearchHit> = hits
        .into_iter()
        .map(|h| SearchHit {
            id: 0,
            score: h.score,
            source_id: h.source_id,
            source_kind: h.source_kind,
            chunk_index: 0,
            text: h.content,
        })
        .collect();

    Ok(out)
}

#[cfg(feature = "ssr")]
fn hash_query(q: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    q.trim().to_lowercase().hash(&mut h);
    h.finish()
}
