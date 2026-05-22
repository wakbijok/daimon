use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("postgres error: {0}")]
    Postgres(#[from] tokio_postgres::Error),

    #[error("refinery error: {0}")]
    Refinery(#[from] refinery::Error),

    #[error("pool error: {0}")]
    Pool(String),
}

pub type Result<T> = std::result::Result<T, Error>;
