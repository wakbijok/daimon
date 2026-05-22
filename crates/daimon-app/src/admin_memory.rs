//! Phase 3 #9 — server-fns backing `/admin/memory`.
//!
//! Two operations:
//! - `admin_memory_ingest` — chunk + embed + upsert text into the tenant's long-term collection
//! - `admin_memory_search` — embed query + vector search + return scored hits
//!
//! Both gated by `require_admin()` (D24). Tenant is currently fixed to `"default"`
//! since multi-tenant primitives land in Phase 2c.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
const DEFAULT_TENANT: &str = "default";
#[cfg(feature = "ssr")]
const QDRANT_URL_ENV: &str = "DAIMON_QDRANT_URL";
#[cfg(feature = "ssr")]
const QDRANT_URL_DEFAULT: &str = "http://localhost:6334";

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

#[cfg(feature = "ssr")]
mod ssr_state {
    use daimon_memory::VectorStore;
    use daimon_rag::Embedder;
    use std::sync::OnceLock;
    use tokio::sync::OnceCell;

    static EMBEDDER: OnceLock<Embedder> = OnceLock::new();
    static STORE: OnceCell<VectorStore> = OnceCell::const_new();

    pub fn embedder() -> Result<&'static Embedder, String> {
        if let Some(e) = EMBEDDER.get() {
            return Ok(e);
        }
        let e = Embedder::new_default().map_err(|err| format!("embedder init: {}", err))?;
        let _ = EMBEDDER.set(e);
        EMBEDDER
            .get()
            .ok_or_else(|| "embedder OnceLock empty after set".to_string())
    }

    pub async fn store() -> Result<&'static VectorStore, String> {
        STORE
            .get_or_try_init(|| async {
                let url = std::env::var(super::QDRANT_URL_ENV)
                    .unwrap_or_else(|_| super::QDRANT_URL_DEFAULT.to_string());
                VectorStore::connect(&url).map_err(|e| format!("qdrant connect: {}", e))
            })
            .await
    }
}

#[server]
pub async fn admin_memory_ingest(req: IngestRequest) -> Result<IngestResult, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;
    use daimon_broker::ActionKind;
    use daimon_rag::{ChunkConfig, Document, ingest_document};
    use std::time::Instant;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();

    let embedder = ssr_state::embedder().map_err(ServerFnError::new)?;
    let store = ssr_state::store().await.map_err(ServerFnError::new)?;

    let doc = Document {
        source_id: req.source_id.clone(),
        source_kind: req.source_kind.clone(),
        content: req.content,
    };

    let collection_target = format!("memory://{}/{}", DEFAULT_TENANT, doc.source_id);
    let start = Instant::now();
    let result = ingest_document(store, embedder, DEFAULT_TENANT, &doc, &ChunkConfig::default())
        .await;
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
            Some(&collection_target),
            Some(&summary),
            latency_ms,
            success,
            vec![
                ("source_id".to_string(), req.source_id.clone()),
                ("source_kind".to_string(), req.source_kind.clone()),
                ("chunks".to_string(), chunks_meta),
                ("tenant".to_string(), DEFAULT_TENANT.to_string()),
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
    use daimon_rag::retrieve;
    use std::time::Instant;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();

    let embedder = ssr_state::embedder().map_err(ServerFnError::new)?;
    let store = ssr_state::store().await.map_err(ServerFnError::new)?;

    let target = format!("memory://{}/_search", DEFAULT_TENANT);
    let start = Instant::now();
    let hits_res = retrieve(store, embedder, DEFAULT_TENANT, &req.query, req.top_k as u64).await;
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
                ("tenant".to_string(), DEFAULT_TENANT.to_string()),
            ],
        )
        .await;

    let hits = hits_res.map_err(|e| ServerFnError::new(format!("retrieve: {}", e)))?;

    let out: Vec<SearchHit> = hits
        .into_iter()
        .map(|h| {
            let p = &h.payload;
            SearchHit {
                id: h.id,
                score: h.score,
                source_id: p
                    .get("source_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                source_kind: p
                    .get("source_kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                chunk_index: p
                    .get("chunk_index")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                text: p
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            }
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
