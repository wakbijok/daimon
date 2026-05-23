use async_trait::async_trait;
use thiserror::Error;
use uuid::Uuid;

use crate::event::{AuditEvent, AuditFilter, NewAuditEvent};

/// The audit log contract. Writes are append-only; the sink implementation
/// is expected to enforce immutability (e.g. via Postgres BEFORE UPDATE/DELETE
/// triggers from migration V005). Queries return ordered events with paging.
#[async_trait]
pub trait AuditSink: Send + Sync {
    /// Append a new event. Returns the assigned id.
    async fn append(&self, event: NewAuditEvent) -> Result<Uuid, AuditError>;

    /// Query events matching the filter. `limit` caps result size;
    /// `offset` skips the first N rows. Results are ordered by `ts DESC`
    /// (newest first) and stable by `id`.
    async fn query(
        &self,
        filter: &AuditFilter,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEvent>, AuditError>;

    /// Total count matching the filter (for paged UI).
    async fn count(&self, filter: &AuditFilter) -> Result<u64, AuditError>;
}

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("audit storage error: {0}")]
    Storage(String),
    #[error("audit serde error: {0}")]
    Serde(String),
    #[error("{0}")]
    Other(String),
}
