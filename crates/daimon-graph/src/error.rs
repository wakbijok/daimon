use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    #[error("cypher error: {code}: {message}")]
    Cypher { code: String, message: String },

    #[error("decode: {0}")]
    Decode(String),

    #[error("schema: {0}")]
    Schema(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
