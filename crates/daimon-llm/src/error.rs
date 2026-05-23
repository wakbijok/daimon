use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("missing API key for provider `{0}` — set the matching env var")]
    MissingApiKey(&'static str),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("provider returned {status}: {body}")]
    ApiError { status: u16, body: String },

    #[error("decode response: {0}")]
    Decode(String),

    #[error("stream error: {0}")]
    Stream(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
