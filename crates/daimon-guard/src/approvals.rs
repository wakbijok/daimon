//! Approval inbox — Postgres-backed.
//!
//! When PolicyEngine returns `RequireApproval`, the Guard creates an
//! `Approval` row and parks the broker.execute call on a condvar with a
//! timeout. Operators approve/deny via `/admin/approvals` UI which writes
//! the status; the Guard wakes up and proceeds (or denies).

use chrono::{DateTime, Utc};
use daimon_db::Pool;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

impl ApprovalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Expired => "expired",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "approved" => Self::Approved,
            "denied" => Self::Denied,
            _ => Self::Expired,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub actor_id: String,
    pub capability: String,
    pub target_ref: Option<String>,
    pub params: serde_json::Value,
    pub status: ApprovalStatus,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decided_by: Option<Uuid>,
}

#[derive(Clone)]
pub struct ApprovalQueue {
    pool: Pool,
}

impl ApprovalQueue {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Create a new pending approval. Returns the new row's id.
    pub async fn enqueue(
        &self,
        tenant_id: Uuid,
        actor_id: &str,
        capability: &str,
        target_ref: Option<&str>,
        params: serde_json::Value,
    ) -> Result<Uuid> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                "INSERT INTO public.approvals
                    (tenant_id, actor_id, capability, target_ref, params, status)
                 VALUES ($1, $2, $3, $4, $5, 'pending')
                 RETURNING id",
                &[&tenant_id, &actor_id, &capability, &target_ref, &params],
            )
            .await?;
        Ok(row.get(0))
    }

    /// Read an approval by id.
    pub async fn get(&self, id: Uuid) -> Result<Option<ApprovalRecord>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "SELECT id, tenant_id, actor_id, capability, target_ref, params,
                        status, created_at, decided_at, decided_by
                 FROM public.approvals WHERE id = $1",
                &[&id],
            )
            .await?;
        Ok(row.map(row_to_record))
    }

    /// Operator decision (approve/deny). Returns the updated record.
    pub async fn decide(
        &self,
        id: Uuid,
        decided_by: Uuid,
        status: ApprovalStatus,
    ) -> Result<ApprovalRecord> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                "UPDATE public.approvals
                 SET status = $1, decided_at = now(), decided_by = $2
                 WHERE id = $3 AND status = 'pending'
                 RETURNING id, tenant_id, actor_id, capability, target_ref, params,
                           status, created_at, decided_at, decided_by",
                &[&status.as_str(), &decided_by, &id],
            )
            .await
            .map_err(|e| Error::Other(format!("decide: {e}")))?;
        Ok(row_to_record(row))
    }

    /// List pending approvals for a tenant. Newest first.
    pub async fn list_pending(&self, tenant_id: Uuid, limit: u32) -> Result<Vec<ApprovalRecord>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, tenant_id, actor_id, capability, target_ref, params,
                        status, created_at, decided_at, decided_by
                 FROM public.approvals
                 WHERE tenant_id = $1 AND status = 'pending'
                 ORDER BY created_at DESC
                 LIMIT $2",
                &[&tenant_id, &(limit as i64)],
            )
            .await?;
        Ok(rows.into_iter().map(row_to_record).collect())
    }

    /// Poll-wait for an approval decision. Returns Ok(status) when decided,
    /// Err(ApprovalTimeout) when the deadline elapses.
    ///
    /// Phase 5: simple poll loop. Phase 5.1 could switch to LISTEN/NOTIFY
    /// for sub-second wakeups.
    pub async fn wait_for_decision(
        &self,
        id: Uuid,
        timeout: std::time::Duration,
        poll_interval: std::time::Duration,
    ) -> Result<ApprovalRecord> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let rec = self.get(id).await?;
            match rec {
                Some(r) if matches!(r.status, ApprovalStatus::Approved | ApprovalStatus::Denied) => {
                    return Ok(r);
                }
                _ => {}
            }
            if std::time::Instant::now() >= deadline {
                // Best-effort mark expired so the inbox UI reflects it.
                let _ = self.expire_if_pending(id).await;
                return Err(Error::ApprovalTimeout(id.to_string()));
            }
            tokio::time::sleep(poll_interval).await;
        }
    }

    async fn expire_if_pending(&self, id: Uuid) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE public.approvals
                 SET status = 'expired', decided_at = now()
                 WHERE id = $1 AND status = 'pending'",
                &[&id],
            )
            .await?;
        Ok(())
    }
}

fn row_to_record(row: tokio_postgres::Row) -> ApprovalRecord {
    let status_str: String = row.get(6);
    ApprovalRecord {
        id: row.get(0),
        tenant_id: row.get(1),
        actor_id: row.get(2),
        capability: row.get(3),
        target_ref: row.get(4),
        params: row.get(5),
        status: ApprovalStatus::from_str(&status_str),
        created_at: row.get(7),
        decided_at: row.get(8),
        decided_by: row.get(9),
    }
}
