//! Query-side retrieval: embed query → vector search → JOIN against Postgres
//! canonical-content tier → return hits with text + score.
//!
//! Phase 3 D5: Qdrant payload no longer carries the chunk text. The point id is
//! the JOIN key against `memory.document_chunks`. This keeps Qdrant memory-light
//! and makes Postgres the single source of truth for displayed content.

use std::collections::HashMap;

use daimon_db::Pool;
use daimon_memory::{Error as MemError, VectorStore};
use qdrant_client::QdrantError;
use uuid::Uuid;

use crate::embedder::Embedder;
use crate::error::{Error, Result};
use crate::ingest::long_term_collection;
use crate::reranker::Reranker;
use crate::sparse::SparseEmbedder;

/// A retrieved chunk — Qdrant score + canonical text from Postgres.
#[derive(Debug, Clone)]
pub struct RetrievedChunk {
    pub chunk_id: u64,
    pub document_id: Uuid,
    pub source_id: String,
    pub source_kind: String,
    pub chunk_index: i32,
    pub content: String,
    pub token_estimate: i32,
    pub score: f32,
}

/// Retrieve `top_k` chunks for a `query` against tenant's long-term collection.
/// Returns hits with content joined in from `memory.document_chunks`.
pub async fn retrieve(
    pool: &Pool,
    store: &VectorStore,
    embedder: &Embedder,
    sparse: &SparseEmbedder,
    tenant_id: Uuid,
    tenant_slug: &str,
    query: &str,
    top_k: u64,
) -> Result<Vec<RetrievedChunk>> {
    let collection = long_term_collection(tenant_slug);
    let dense_vec = embedder
        .embed(&[query])?
        .into_iter()
        .next()
        .expect("one dense embedding");
    let sparse_vec = sparse
        .embed(&[query])?
        .into_iter()
        .next()
        .expect("one sparse embedding");

    let qdrant_hits = match store
        .query_hybrid(
            &collection,
            dense_vec,
            sparse_vec.indices,
            sparse_vec.values,
            top_k,
        )
        .await
    {
        Ok(hits) => hits,
        Err(e) if is_not_found(&e) => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };

    if qdrant_hits.is_empty() {
        return Ok(vec![]);
    }

    // Score-by-id map so we can re-apply the order after the Postgres JOIN.
    let mut score_by_id: HashMap<u64, f32> = HashMap::new();
    let mut chunk_ids_i64: Vec<i64> = Vec::with_capacity(qdrant_hits.len());
    for h in &qdrant_hits {
        score_by_id.insert(h.id, h.score);
        chunk_ids_i64.push(h.id as i64);
    }

    let client = pool.get().await.map_err(|e| Error::Other(format!("pg pool: {e}")))?;
    let rows = client
        .query(
            "SELECT dc.id, dc.document_id, d.source_id, d.source_kind,
                    dc.chunk_index, dc.content, dc.token_estimate
             FROM memory.document_chunks dc
             JOIN memory.documents d ON d.id = dc.document_id
             WHERE dc.tenant_id = $1 AND dc.id = ANY($2::BIGINT[])",
            &[&tenant_id, &chunk_ids_i64],
        )
        .await
        .map_err(|e| Error::Other(format!("chunk join: {e}")))?;

    let mut hits: Vec<RetrievedChunk> = rows
        .into_iter()
        .filter_map(|r| {
            let id_i64: i64 = r.get(0);
            let id = id_i64 as u64;
            let score = *score_by_id.get(&id)?;
            Some(RetrievedChunk {
                chunk_id: id,
                document_id: r.get(1),
                source_id: r.get(2),
                source_kind: r.get(3),
                chunk_index: r.get(4),
                content: r.get(5),
                token_estimate: r.get(6),
                score,
            })
        })
        .collect();

    // Restore the Qdrant ordering — JOINs are unordered.
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    Ok(hits)
}

/// Retrieve with cross-encoder rerank applied after the hybrid search.
///
/// Pulls a wider candidate set (`prefetch_k`, typically 4× the requested
/// `top_k`) from hybrid retrieval, then reranks all candidates against the
/// query via [`Reranker`] and returns the top `top_k` by rerank score.
/// Reranker scores REPLACE the original Qdrant fusion scores in the output.
pub async fn retrieve_with_rerank(
    pool: &Pool,
    store: &VectorStore,
    embedder: &Embedder,
    sparse: &SparseEmbedder,
    reranker: &Reranker,
    tenant_id: Uuid,
    tenant_slug: &str,
    query: &str,
    top_k: u64,
    prefetch_k: u64,
) -> Result<Vec<RetrievedChunk>> {
    let candidates = retrieve(
        pool, store, embedder, sparse, tenant_id, tenant_slug, query, prefetch_k,
    )
    .await?;
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let docs: Vec<&str> = candidates.iter().map(|c| c.content.as_str()).collect();
    let scores = reranker.score(query, &docs)?;

    let mut reranked: Vec<RetrievedChunk> = candidates
        .into_iter()
        .zip(scores.into_iter())
        .map(|(mut c, s)| {
            c.score = s;
            c
        })
        .collect();
    reranked.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    reranked.truncate(top_k as usize);
    Ok(reranked)
}

fn is_not_found(e: &MemError) -> bool {
    let MemError::Qdrant(QdrantError::ResponseError { status }) = e else {
        return false;
    };
    status.code() == tonic::Code::NotFound
}
