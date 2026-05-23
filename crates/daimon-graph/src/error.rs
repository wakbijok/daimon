use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("bolt: {0}")]
    Bolt(#[from] neo4rs::Error),

    #[error("decode: {0}")]
    Decode(String),

    #[error("schema: {0}")]
    Schema(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
