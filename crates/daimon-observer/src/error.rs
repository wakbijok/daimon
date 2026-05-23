use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    #[error("pg: {0}")]
    Pg(#[from] tokio_postgres::Error),

    #[error("pool: {0}")]
    Pool(String),

    #[error("decode: {0}")]
    Decode(String),

    #[error("prometheus api error: {0}")]
    Api(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<deadpool_postgres::PoolError> for Error {
    fn from(e: deadpool_postgres::PoolError) -> Self {
        Error::Pool(e.to_string())
    }
}
