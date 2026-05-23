use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("platform not found: {0}")]
    PlatformNotFound(String),

    #[error("workload not found: {0}")]
    WorkloadNotFound(String),

    #[error("capability not supported: {0}")]
    CapabilityNotSupported(String),

    #[error("driver: {0}")]
    Driver(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<daimon_pve::Error> for Error {
    fn from(e: daimon_pve::Error) -> Self {
        Error::Driver(format!("pve: {e}"))
    }
}
