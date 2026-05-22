//! Document ingestion: chunk → embed → upsert to the Qdrant collection.
//!
//! Phase 3 D6-lite: vector-only ingest. Sparse vectors (BM25) and canonical-payload
//! Postgres storage land in the full D6 + Phase 2c respectively. Tenant isolation
//! is enforced here by mapping `tenant_id` to a collection name `tenant_<id>_<purpose>`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use daimon_memory::{Point, VectorStore};
use serde_json::json;

use crate::chunker::{Chunk, ChunkConfig, chunk};
use crate::embedder::Embedder;
use crate::error::Result;

/// A unit of content to ingest. Caller supplies a stable `source_id` so re-ingesting
/// the same source overwrites (point IDs are derived from a hash of `(source_id, chunk_index)`).
#[derive(Debug, Clone)]
pub struct Document {
    pub source_id: String,
    pub source_kind: String,
    pub content: String,
}

/// Result of an ingest call.
#[derive(Debug, Clone)]
pub struct IngestStats {
    pub source_id: String,
    pub chunks: usize,
    pub collection: String,
}

/// Standard collection naming for a tenant's long-term memory.
pub fn long_term_collection(tenant_id: &str) -> String {
    format!("tenant_{}_long_term", tenant_id)
}

/// Stable u64 ID for a chunk: hash of `(source_id || ':' || chunk_index)`.
pub fn chunk_point_id(source_id: &str, chunk_index: usize) -> u64 {
    let mut h = DefaultHasher::new();
    source_id.hash(&mut h);
    ':'.hash(&mut h);
    chunk_index.hash(&mut h);
    h.finish()
}

/// Chunk, embed, and upsert one document into the tenant's long-term collection.
pub async fn ingest_document(
    store: &VectorStore,
    embedder: &Embedder,
    tenant_id: &str,
    doc: &Document,
    chunk_cfg: &ChunkConfig,
) -> Result<IngestStats> {
    let collection = long_term_collection(tenant_id);
    store.ensure_collection(&collection, embedder.dim() as u64).await?;

    let chunks: Vec<Chunk> = chunk(&doc.content, chunk_cfg);
    if chunks.is_empty() {
        return Ok(IngestStats {
            source_id: doc.source_id.clone(),
            chunks: 0,
            collection,
        });
    }

    let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
    let vectors = embedder.embed(&texts)?;

    let points: Vec<Point> = chunks
        .iter()
        .zip(vectors.into_iter())
        .map(|(c, v)| {
            let id = chunk_point_id(&doc.source_id, c.index);
            let payload = json!({
                "tenant_id": tenant_id,
                "source_id": doc.source_id,
                "source_kind": doc.source_kind,
                "chunk_index": c.index,
                "word_start": c.word_start,
                "word_end": c.word_end,
                "text": c.text,
            });
            Point {
                id,
                vector: v,
                payload,
            }
        })
        .collect();

    let chunks_count = points.len();
    store.upsert(&collection, points).await?;

    Ok(IngestStats {
        source_id: doc.source_id.clone(),
        chunks: chunks_count,
        collection,
    })
}
