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
    use daimon_rag::{ChunkConfig, Document, ingest_document};

    require_admin().await?;

    let embedder = ssr_state::embedder().map_err(ServerFnError::new)?;
    let store = ssr_state::store().await.map_err(ServerFnError::new)?;

    let doc = Document {
        source_id: req.source_id.clone(),
        source_kind: req.source_kind.clone(),
        content: req.content,
    };

    let stats = ingest_document(store, embedder, DEFAULT_TENANT, &doc, &ChunkConfig::default())
        .await
        .map_err(|e| ServerFnError::new(format!("ingest: {}", e)))?;

    Ok(IngestResult {
        source_id: stats.source_id,
        chunks: stats.chunks,
        collection: stats.collection,
    })
}

#[server]
pub async fn admin_memory_search(req: SearchRequest) -> Result<Vec<SearchHit>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use daimon_rag::retrieve;

    require_admin().await?;

    let embedder = ssr_state::embedder().map_err(ServerFnError::new)?;
    let store = ssr_state::store().await.map_err(ServerFnError::new)?;

    let hits = retrieve(store, embedder, DEFAULT_TENANT, &req.query, req.top_k as u64)
        .await
        .map_err(|e| ServerFnError::new(format!("retrieve: {}", e)))?;

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
