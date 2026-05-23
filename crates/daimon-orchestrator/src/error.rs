use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("db: {0}")]
    Pg(#[from] tokio_postgres::Error),

    #[error("pool: {0}")]
    Pool(String),

    #[error("plan not found: {0}")]
    PlanNotFound(uuid::Uuid),

    #[error("step dispatch: {0}")]
    Dispatch(String),

    #[error("dag cycle detected")]
    DagCycle,

    #[error("decode: {0}")]
    Decode(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<deadpool_postgres::PoolError> for Error {
    fn from(e: deadpool_postgres::PoolError) -> Self {
        Error::Pool(e.to_string())
    }
}
