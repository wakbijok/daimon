//! Local text embedding via `fastembed` (ONNX runtime, CPU by default).
//!
//! Default model: `BGESmallENV15` (BAAI/bge-small-en-v1.5, 384-dim). Models are downloaded
//! lazily on first instantiation into `~/.cache/fastembed/` (configurable via `cache_dir`).
//!
//! `fastembed::TextEmbedding::embed` requires `&mut self`, so [`Embedder`] is built around
//! a parking_lot::Mutex (added when needed for concurrent callers). For Phase 3 D4 we expose
//! the simpler single-caller wrapper; concurrency-shaped wrapping arrives when D6's ingest
//! pipeline lands and we know the actual access pattern.

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use std::sync::Mutex;

use crate::error::{Error, Result};

/// Wraps a `fastembed::TextEmbedding` model. Holds the model behind a `Mutex` so the embed
/// call can take `&self` (the underlying ONNX session needs `&mut`).
pub struct Embedder {
    inner: Mutex<TextEmbedding>,
    model: EmbeddingModel,
    dim: usize,
}

impl Embedder {
    /// Construct with the default model (`BGESmallENV15`, 384-dim). Downloads model on first use.
    pub fn new_default() -> Result<Self> {
        Self::new(EmbeddingModel::BGESmallENV15)
    }

    /// Construct with a specific embedding model.
    pub fn new(model: EmbeddingModel) -> Result<Self> {
        let dim = match TextEmbedding::get_model_info(&model) {
            Ok(info) => info.dim,
            Err(e) => return Err(Error::Embedding(format!("model info: {}", e))),
        };
        let opts = TextInitOptions::new(model.clone());
        let inner = TextEmbedding::try_new(opts)
            .map_err(|e| Error::Embedding(format!("init {:?}: {}", model, e)))?;
        Ok(Self {
            inner: Mutex::new(inner),
            model,
            dim,
        })
    }

    /// Output dimension of the embedding vectors.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// The model variant in use.
    pub fn model(&self) -> &EmbeddingModel {
        &self.model
    }

    /// Embed a batch of texts. Returns one vector per input.
    pub fn embed<S: AsRef<str> + Send + Sync>(&self, texts: &[S]) -> Result<Vec<Vec<f32>>> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| Error::Embedding(format!("mutex poisoned: {}", e)))?;
        let out = guard
            .embed(texts, None)
            .map_err(|e| Error::Embedding(format!("embed: {}", e)))?;
        Ok(out)
    }
}

/// Cosine similarity between two equally-sized vectors. Returns `0.0` if either norm is zero.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "vector dim mismatch");
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identity() {
        let v = vec![1.0, 2.0, 3.0];
        let s = cosine(&v, &v);
        assert!((s - 1.0).abs() < 1e-5, "self-cosine should be 1.0, got {}", s);
    }

    #[test]
    fn cosine_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let s = cosine(&a, &b);
        assert!(s.abs() < 1e-5, "orthogonal cosine should be 0, got {}", s);
    }
}
