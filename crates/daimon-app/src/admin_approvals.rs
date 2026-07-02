//! Phase 8 — server-fns backing `/admin/approvals`.
//!
//! Two endpoints:
//! - `list_pending_approvals_with_blast_radius()` — pending inbox rows for
//!   this tenant, each enriched with a NornicDB blast-radius summary
//!   (empty list if the graph tier is disabled or the lookup failed).
//! - `decide_approval(id, approved, comment)` — operator decision. Writes
//!   the row to `approved` or `denied`; the broker's parked
//!   `wait_for_decision` loop wakes up and proceeds (or returns the
//!   `PolicyDenied` error path).

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRow {
    pub id: String,
    pub actor_id: String,
    pub capability: String,
    pub target_ref: Option<String>,
    pub params_pretty: String,
    pub created_at: String,
    pub blast_radius: Vec<BlastRadiusItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlastRadiusItem {
    pub kind: String,
    pub id: String,
    pub label: String,
    pub depth: u32,
}

#[server]
pub async fn list_pending_approvals_with_blast_radius(
    limit: Option<u32>,
) -> Result<Vec<ApprovalRow>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;
    use daimon_graph::TargetRef as GraphTargetRef;

    let _claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let lim = limit.unwrap_or(50);

    // Read the pending inbox via the broker's admin-side ApprovalQueue
    // accessor. The broker holds the Guard which holds the queue.
    let approvals = state
        .broker
        .approvals_pending(lim)
        .await
        .map_err(|e| ServerFnError::new(format!("list_pending: {e}")))?;

    let mut out = Vec::with_capacity(approvals.len());
    for a in approvals {
        let params_pretty = serde_json::to_string_pretty(&a.params).unwrap_or_default();
        let blast_radius = match (state.graph.as_ref(), a.target_ref.as_deref()) {
            (Some(g), Some(tref)) => match g
                .blast_radius(
                    &GraphTargetRef::from(tref),
                    daimon_guard::DEFAULT_BLAST_RADIUS_DEPTH,
                )
                .await
            {
                Ok(entries) => entries
                    .into_iter()
                    .map(|e| BlastRadiusItem {
                        kind: format!("{:?}", e.kind),
                        id: e.id,
                        label: e.label,
                        depth: e.depth,
                    })
                    .collect(),
                Err(e) => {
                    tracing::warn!(target = %tref, error = %e, "blast_radius lookup failed");
                    Vec::new()
                }
            },
            _ => Vec::new(),
        };
        out.push(ApprovalRow {
            id: a.id.to_string(),
            actor_id: a.actor_id,
            capability: a.capability,
            target_ref: a.target_ref,
            params_pretty,
            created_at: a.created_at.to_rfc3339(),
            blast_radius,
        });
    }
    Ok(out)
}

#[server]
pub async fn decide_approval(
    id: String,
    approved: bool,
) -> Result<String, ServerFnError> {
    use crate::auth_guard::require_approver;
    use crate::state::AppState;
    use daimon_guard::ApprovalStatus;

    let claims = require_approver().await?;
    let state = expect_context::<AppState>();
    let approval_id = uuid::Uuid::parse_str(&id)
        .map_err(|e| ServerFnError::new(format!("parse id: {e}")))?;
    let status = if approved { ApprovalStatus::Approved } else { ApprovalStatus::Denied };
    let rec = state
        .broker
        .approvals_decide(approval_id, claims.user_id, status)
        .await
        .map_err(|e| ServerFnError::new(format!("decide: {e}")))?;
    Ok(format!("{:?}", rec.status).to_lowercase())
}
