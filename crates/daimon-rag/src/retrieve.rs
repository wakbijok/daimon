//! Query-side retrieval: embed query → vector search → return scored hits.
//!
//! Phase 3 D6-lite: dense-vector only. Sparse / hybrid / rerank land in full D6.
//! Tenant scope is enforced via collection naming
//! (see [`crate::ingest::long_term_collection`]).

use daimon_memory::{Error as MemError, Hit, VectorStore};
use qdrant_client::QdrantError;

use crate::embedder::Embedder;
use crate::error::Result;
use crate::ingest::long_term_collection;

/// Retrieve `top_k` chunks for a `query` against tenant's long-term collection.
///
/// Returns an empty `Vec` if the tenant has no long-term collection yet (e.g. never ingested
/// anything). Other Qdrant errors propagate.
pub async fn retrieve(
    store: &VectorStore,
    embedder: &Embedder,
    tenant_id: &str,
    query: &str,
    top_k: u64,
) -> Result<Vec<Hit>> {
    let collection = long_term_collection(tenant_id);
    let query_vec = embedder
        .embed(&[query])?
        .into_iter()
        .next()
        .expect("one embedding");
    match store.search(&collection, query_vec, top_k).await {
        Ok(hits) => Ok(hits),
        Err(e) if is_not_found(&e) => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}

fn is_not_found(e: &MemError) -> bool {
    let MemError::Qdrant(QdrantError::ResponseError { status }) = e else {
        return false;
    };
    status.code() == tonic::Code::NotFound
}
