use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("memory error: {0}")]
    Memory(#[from] daimon_memory::Error),

    #[error("embedding error: {0}")]
    Embedding(String),
}

pub type Result<T> = std::result::Result<T, Error>;
