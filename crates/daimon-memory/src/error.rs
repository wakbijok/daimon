use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("qdrant client error: {0}")]
    Qdrant(#[from] qdrant_client::QdrantError),

    #[error("collection not found: {0}")]
    CollectionNotFound(String),

    #[error("invalid dimension: expected {expected}, got {got}")]
    InvalidDimension { expected: usize, got: usize },

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
