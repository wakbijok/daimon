use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("redis: {0}")]
    Redis(#[from] deadpool_redis::redis::RedisError),

    #[error("pool: {0}")]
    Pool(String),

    #[error("decode: {0}")]
    Decode(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<deadpool_redis::PoolError> for Error {
    fn from(e: deadpool_redis::PoolError) -> Self {
        Error::Pool(e.to_string())
    }
}
