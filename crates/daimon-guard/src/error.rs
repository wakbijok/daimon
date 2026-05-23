use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("pool: {0}")]
    Pool(String),

    #[error("pg: {0}")]
    Pg(#[from] tokio_postgres::Error),

    #[error("decode: {0}")]
    Decode(String),

    #[error("kill switch engaged: {reason}")]
    KillEngaged { reason: String },

    #[error("policy denied: {reason}")]
    PolicyDenied { reason: String },

    #[error("approval required: {capability}")]
    ApprovalRequired { capability: String },

    #[error("approval timeout: {0}")]
    ApprovalTimeout(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<deadpool_postgres::PoolError> for Error {
    fn from(e: deadpool_postgres::PoolError) -> Self {
        Error::Pool(e.to_string())
    }
}
