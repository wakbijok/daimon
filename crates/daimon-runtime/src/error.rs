use daimon_core::CoreError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("agent `{0}` is already registered")]
    DuplicateAgent(String),

    #[error("agent `{0}` panicked: {1}")]
    AgentPanic(String, String),

    #[error("supervisor shutdown")]
    Shutdown,

    #[error(transparent)]
    Core(#[from] CoreError),

    #[error("{0}")]
    Other(String),
}
