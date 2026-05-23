//! daimon-rag — retrieval-augmented context assembly.
//!
//! Pipeline:
//! 1. **chunker** — token-aware splitting (default 512/64 overlap, sentence-boundary aware)
//! 2. **embedder** — local dense embeddings via `fastembed` (bge-small-en-v1.5, 384d)
//! 3. **sparse** — local SPLADE++ sparse embeddings for lexical signal
//! 4. **ingest** — chunk → embed (dense+sparse) → write canonical text to Postgres
//!    `memory.document_chunks` + upsert named vectors to Qdrant
//! 5. **retrieve** — query → embed dense+sparse → Qdrant hybrid query (RRF) → JOIN
//!    canonical text from Postgres → return scored hits
//! 6. **reranker** — cross-encoder rerank (bge-reranker-base) over top-K
//! 7. **context_pack** — greedy-MMR pack to a caller-supplied token budget
//!
//! Phase 3 D5+D6 ships the full hybrid + rerank + pack pipeline. Canonical content
//! lives in Postgres (RLS-enforced); Qdrant holds embeddings + lightweight metadata.

pub mod chunker;
pub mod context_pack;
pub mod embedder;
pub mod error;
pub mod ingest;
pub mod reranker;
pub mod retrieve;
pub mod sparse;

pub use chunker::{Chunk, ChunkConfig, chunk};
pub use context_pack::{ContextItem, PackConfig, pack_context};
pub use embedder::{Embedder, cosine};
pub use error::{Error, Result};
pub use ingest::{Document, IngestStats, delete_document, ingest_document, long_term_collection};
pub use reranker::Reranker;
pub use retrieve::{RetrievedChunk, retrieve, retrieve_with_rerank};
pub use sparse::{SparseEmbedder, SparseVector};
