//! Sparse-vector embedding via SPLADE++ (fastembed `SparseTextEmbedding`).
//!
//! Sparse vectors complement the dense BGE embedding by giving Qdrant the
//! lexical / BM25-like signal. Hybrid retrieval issues two prefetch queries
//! (dense + sparse) and fuses them with Reciprocal Rank Fusion (RRF).
//!
//! Default model: SPLADE++ v1 (`Qdrant/Splade_PP_en_v1`). Lazy model download
//! on first instantiation; the model files cache under `~/.cache/fastembed/`
//! by default (verify via `FASTEMBED_CACHE`).

use std::sync::Mutex;

use fastembed::{SparseInitOptions, SparseModel, SparseTextEmbedding};

use crate::error::{Error, Result};

/// Sparse vector — (term-index, weight) pairs. Qdrant takes this as
/// `(Vec<u32>, Vec<f32>)` of equal length.
#[derive(Debug, Clone, Default)]
pub struct SparseVector {
    pub indices: Vec<u32>,
    pub values: Vec<f32>,
}

pub struct SparseEmbedder {
    inner: Mutex<SparseTextEmbedding>,
}

impl SparseEmbedder {
    /// Default — SPLADE++ v1.
    pub fn new_default() -> Result<Self> {
        let opts = SparseInitOptions::new(SparseModel::SPLADEPPV1).with_show_download_progress(true);
        let inner = SparseTextEmbedding::try_new(opts)
            .map_err(|e| Error::Embedding(format!("sparse init: {e}")))?;
        Ok(Self {
            inner: Mutex::new(inner),
        })
    }

    /// Embed a batch of texts into sparse vectors.
    pub fn embed(&self, texts: &[&str]) -> Result<Vec<SparseVector>> {
        let docs: Vec<String> = texts.iter().map(|t| t.to_string()).collect();
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| Error::Embedding(format!("sparse mutex poisoned: {e}")))?;
        let out = guard
            .embed(docs, None)
            .map_err(|e| Error::Embedding(format!("sparse embed: {e}")))?;
        Ok(out
            .into_iter()
            .map(|e| SparseVector {
                indices: e.indices.iter().map(|&i| i as u32).collect(),
                values: e.values,
            })
            .collect())
    }
}
