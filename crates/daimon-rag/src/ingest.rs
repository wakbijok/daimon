//! Document ingestion: chunk → write canonical to Postgres → embed → upsert to Qdrant.
//!
//! Phase 3 D5: `memory.documents` + `memory.document_chunks` are the source-of-truth
//! payload tier. Qdrant payload no longer carries the full text — just lightweight
//! references (source_id, source_kind, chunk_index). Retrieval JOINs Qdrant hits
//! against `memory.document_chunks` for the actual content.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use daimon_db::Pool;
use daimon_memory::{HybridPoint, VectorStore};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::chunker::{Chunk, ChunkConfig, chunk};
use crate::embedder::Embedder;
use crate::error::{Error, Result};
use crate::sparse::SparseEmbedder;

/// A unit of content to ingest. Caller supplies a stable `source_id` so re-ingesting
/// the same source overwrites by `source_id`.
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
    pub document_id: Uuid,
    pub chunks: usize,
    pub collection: String,
    pub skipped_unchanged: bool,
}

/// The single long-term memory collection (single-org). Returns the fixed
/// Qdrant collection name.
pub fn long_term_collection() -> String {
    daimon_memory::COLLECTION.to_string()
}

/// Stable u64 ID for a chunk: hash of `(source_id || ':' || chunk_index)`.
pub fn chunk_point_id(source_id: &str, chunk_index: usize) -> u64 {
    let mut h = DefaultHasher::new();
    source_id.hash(&mut h);
    ':'.hash(&mut h);
    chunk_index.hash(&mut h);
    h.finish()
}

fn token_estimate(text: &str) -> i32 {
    // Rough heuristic: 1 token ≈ 4 chars OR whitespace-split count, whichever is larger.
    let by_chars = (text.len() as f32 / 4.0).ceil() as i32;
    let by_words = text.split_whitespace().count() as i32;
    by_chars.max(by_words)
}

/// Ingest pipeline:
/// 1. Compute content hash; skip if existing `memory.documents` row has the same hash
/// 2. UPSERT the document header into `memory.documents`
/// 3. DELETE all existing chunks for the document (re-ingest replaces)
/// 4. INSERT chunks into `memory.document_chunks` with id = chunk_point_id
/// 5. Embed all chunks
/// 6. Upsert (id, vector, lightweight-payload) into Qdrant
pub async fn ingest_document(
    pool: &Pool,
    store: &VectorStore,
    embedder: &Embedder,
    sparse: &SparseEmbedder,
    doc: &Document,
    chunk_cfg: &ChunkConfig,
) -> Result<IngestStats> {
    let mut client = pool.get().await.map_err(|e| Error::Other(format!("pg pool: {e}")))?;

    let content_hash = {
        let mut h = Sha256::new();
        h.update(doc.content.as_bytes());
        h.finalize().to_vec()
    };

    // 1. Look up existing document.
    let existing = client
        .query_opt(
            "SELECT id, content_hash FROM memory.documents
             WHERE source_id = $1",
            &[&doc.source_id],
        )
        .await
        .map_err(|e| Error::Other(format!("documents lookup: {e}")))?;

    let collection = long_term_collection();

    if let Some(ref row) = existing {
        let existing_hash: Vec<u8> = row.get(1);
        if existing_hash == content_hash {
            let doc_id: Uuid = row.get(0);
            tracing::info!(
                target: "rag.ingest",
                source_id = %doc.source_id,
                "unchanged content hash — skipping re-ingest"
            );
            return Ok(IngestStats {
                source_id: doc.source_id.clone(),
                document_id: doc_id,
                chunks: 0,
                collection,
                skipped_unchanged: true,
            });
        }
    }

    // Chunk + token estimates first so we have everything ready before opening the txn.
    let chunks: Vec<Chunk> = chunk(&doc.content, chunk_cfg);
    if chunks.is_empty() {
        // Empty doc — still record a header, no chunks.
        let doc_id = upsert_document(&mut client, doc, &content_hash).await?;
        return Ok(IngestStats {
            source_id: doc.source_id.clone(),
            document_id: doc_id,
            chunks: 0,
            collection,
            skipped_unchanged: false,
        });
    }

    // 2-4. Postgres transaction: upsert document, delete old chunks, insert new chunks.
    let txn = client
        .transaction()
        .await
        .map_err(|e| Error::Other(format!("begin txn: {e}")))?;

    let doc_id = upsert_document_txn(&txn, doc, &content_hash).await?;
    txn.execute(
        "DELETE FROM memory.document_chunks WHERE document_id = $1",
        &[&doc_id],
    )
    .await
    .map_err(|e| Error::Other(format!("delete old chunks: {e}")))?;

    for c in &chunks {
        let point_id = chunk_point_id(&doc.source_id, c.index);
        // Cast u64 → i64 (Postgres BIGINT). Upper bit becomes sign — that's fine,
        // equality is preserved.
        let point_id_i64 = point_id as i64;
        txn.execute(
            "INSERT INTO memory.document_chunks
                (id, document_id, chunk_index, content, word_start, word_end, token_estimate)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &point_id_i64,
                &doc_id,
                &(c.index as i32),
                &c.text,
                &(c.word_start as i32),
                &(c.word_end as i32),
                &token_estimate(&c.text),
            ],
        )
        .await
        .map_err(|e| Error::Other(format!("insert chunk: {e}")))?;
    }
    txn.commit()
        .await
        .map_err(|e| Error::Other(format!("commit: {e}")))?;

    // 5-6. Embed (dense + sparse) + upsert hybrid points to Qdrant.
    store
        .ensure_hybrid_collection(&collection, embedder.dim() as u64)
        .await?;

    let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
    let dense_vectors = embedder.embed(&texts)?;
    let sparse_vectors = sparse.embed(&texts)?;

    let points: Vec<HybridPoint> = chunks
        .iter()
        .zip(dense_vectors.into_iter())
        .zip(sparse_vectors.into_iter())
        .map(|((c, d), s)| {
            let id = chunk_point_id(&doc.source_id, c.index);
            let payload = json!({
                "document_id": doc_id.to_string(),
                "source_id": doc.source_id,
                "source_kind": doc.source_kind,
                "chunk_index": c.index,
            });
            HybridPoint {
                id,
                dense: d,
                sparse_indices: s.indices,
                sparse_values: s.values,
                payload,
            }
        })
        .collect();

    let chunks_count = points.len();
    store.upsert_hybrid(&collection, points).await?;

    Ok(IngestStats {
        source_id: doc.source_id.clone(),
        document_id: doc_id,
        chunks: chunks_count,
        collection,
        skipped_unchanged: false,
    })
}

/// Delete a document + all its chunks + the corresponding Qdrant points.
pub async fn delete_document(
    pool: &Pool,
    store: &VectorStore,
    source_id: &str,
) -> Result<usize> {
    let client = pool.get().await.map_err(|e| Error::Other(format!("pg pool: {e}")))?;
    // Fetch chunk ids before we delete them so we can also drop them from Qdrant.
    let rows = client
        .query(
            "SELECT dc.id FROM memory.document_chunks dc
             JOIN memory.documents d ON d.id = dc.document_id
             WHERE d.source_id = $1",
            &[&source_id],
        )
        .await
        .map_err(|e| {
            Error::Other(format!(
                "fetch chunk ids: {e}: db={:?}",
                e.as_db_error()
            ))
        })?;
    let chunk_ids: Vec<u64> = rows
        .into_iter()
        .map(|r| {
            let id: i64 = r.get(0);
            id as u64
        })
        .collect();
    let n = chunk_ids.len();

    // Postgres delete cascades to chunks via FK.
    client
        .execute(
            "DELETE FROM memory.documents WHERE source_id = $1",
            &[&source_id],
        )
        .await
        .map_err(|e| Error::Other(format!("delete document: {e}")))?;

    // Qdrant — best effort. Errors logged but not propagated; the source of truth
    // is Postgres, so a leaked Qdrant point is auditable and harmless (no canonical text).
    if !chunk_ids.is_empty() {
        let collection = long_term_collection();
        if let Err(e) = store.delete_points(&collection, &chunk_ids).await {
            tracing::warn!(
                target: "rag.delete",
                error = %e,
                collection,
                "qdrant point delete failed — postgres state is canonical"
            );
        }
    }

    Ok(n)
}

async fn upsert_document(
    client: &mut deadpool_postgres::Client,
    doc: &Document,
    content_hash: &[u8],
) -> Result<Uuid> {
    let row = client
        .query_one(
            "INSERT INTO memory.documents (source_id, source_kind, content_hash)
             VALUES ($1, $2, $3)
             ON CONFLICT (source_id) DO UPDATE
                SET source_kind = EXCLUDED.source_kind,
                    content_hash = EXCLUDED.content_hash,
                    updated_at = now()
             RETURNING id",
            &[&doc.source_id, &doc.source_kind, &content_hash],
        )
        .await
        .map_err(|e| Error::Other(format!("upsert document: {e}")))?;
    Ok(row.get(0))
}

async fn upsert_document_txn(
    txn: &deadpool_postgres::Transaction<'_>,
    doc: &Document,
    content_hash: &[u8],
) -> Result<Uuid> {
    let row = txn
        .query_one(
            "INSERT INTO memory.documents (source_id, source_kind, content_hash)
             VALUES ($1, $2, $3)
             ON CONFLICT (source_id) DO UPDATE
                SET source_kind = EXCLUDED.source_kind,
                    content_hash = EXCLUDED.content_hash,
                    updated_at = now()
             RETURNING id",
            &[&doc.source_id, &doc.source_kind, &content_hash],
        )
        .await
        .map_err(|e| Error::Other(format!("upsert document: {e}")))?;
    Ok(row.get(0))
}
