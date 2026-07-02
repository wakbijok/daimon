use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// The sidecar answered with a non-2xx status.
    #[error("dmem http error: {0}")]
    Http(String),

    /// The sidecar's response body could not be decoded into the expected shape.
    #[error("dmem decode error: {0}")]
    Decode(String),

    /// The sidecar could not be reached (connect refused, DNS, TLS, timeout).
    #[error("dmem unreachable: {0}")]
    Unreachable(String),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
