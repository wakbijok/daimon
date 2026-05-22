//! daimon-rag — retrieval-augmented context assembly.
//!
//! Pipeline:
//! 1. **chunker** — token-aware splitting (default 512/64 overlap, sentence-boundary aware)
//! 2. **embedder** — local embeddings via `fastembed` (bge-small-en-v1.5 default, 384d)
//! 3. **ingest** — chunk + embed + upsert to Qdrant via [`daimon_memory::VectorStore`]
//! 4. **retrieve** — query → embed → hybrid search → optional rerank → ranked hits
//! 5. **context_pack** — greedy-MMR pack to a caller-supplied token budget
//!
//! Phase 3 lands embedder + a single-vector dense ingest/retrieve path first. Sparse / hybrid
//! and reranker land in later deliverables (D6) per `daimon-docs/plans/2026-05-23-phase-3-memory-rag-qdrant-plan.md`.

pub mod chunker;
pub mod embedder;
pub mod error;
pub mod ingest;
pub mod retrieve;

pub use chunker::{Chunk, ChunkConfig, chunk};
pub use embedder::{Embedder, cosine};
pub use error::{Error, Result};
pub use ingest::{Document, IngestStats, ingest_document, long_term_collection};
pub use retrieve::retrieve;
