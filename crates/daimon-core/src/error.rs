use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("no agent registered for capability `{name}` matching `{version_req}`")]
    CapabilityNotFound { name: String, version_req: String },

    #[error("agent `{0}` not found")]
    AgentNotFound(String),

    #[error("bus send failed: {0}")]
    BusSend(String),

    #[error("agent handler error: {0}")]
    Handler(String),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}
