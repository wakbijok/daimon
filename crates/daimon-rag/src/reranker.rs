//! Cross-encoder reranker — scores (query, passage) pairs and reorders.
//!
//! Phase 3 D6b: wraps `fastembed::TextRerank` with the BGE-reranker-base model
//! (the default). Lazy model download on first instantiation; ~110 MB on disk.
//!
//! Typical use: retrieve top-K from hybrid search (K = 25), rerank, take the
//! top-N (N = 5-10) for the LLM context.

use std::sync::Mutex;

use fastembed::{RerankInitOptions, RerankerModel, TextRerank};

use crate::error::{Error, Result};

pub struct Reranker {
    inner: Mutex<TextRerank>,
}

impl Reranker {
    /// Default — BAAI/bge-reranker-base (English+Chinese).
    pub fn new_default() -> Result<Self> {
        let opts = RerankInitOptions::new(RerankerModel::BGERerankerBase)
            .with_show_download_progress(true);
        let inner = TextRerank::try_new(opts)
            .map_err(|e| Error::Embedding(format!("reranker init: {e}")))?;
        Ok(Self {
            inner: Mutex::new(inner),
        })
    }

    /// Score each `(query, doc)` pair. Returns scores in the same order as
    /// `docs`. Higher is more relevant.
    pub fn score(&self, query: &str, docs: &[&str]) -> Result<Vec<f32>> {
        if docs.is_empty() {
            return Ok(Vec::new());
        }
        let docs_owned: Vec<String> = docs.iter().map(|d| d.to_string()).collect();
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| Error::Embedding(format!("reranker mutex poisoned: {e}")))?;
        let q = query.to_string();
        let results = guard
            .rerank(q, &docs_owned, true, None)
            .map_err(|e| Error::Embedding(format!("rerank: {e}")))?;
        // fastembed's rerank() returns results sorted by score desc, each
        // carrying an `index` back-pointer into the input order. Reassemble
        // into input order.
        let mut scores = vec![0.0f32; docs.len()];
        for r in results {
            if r.index < scores.len() {
                scores[r.index] = r.score;
            }
        }
        Ok(scores)
    }
}
