//! Approval-time blast-radius enrichment.
//!
//! When the approval inbox UI renders a pending action, it asks the graph
//! tier "what depends on this target?" so the operator sees the impact
//! before approving. The graph (NornicDB) is queried via the
//! `daimon-graph::GraphClient` trait — implementation-agnostic.
//!
//! Best-effort: a graph-side failure logs and returns an empty list. The
//! approval flow proceeds without the summary rather than blocking on
//! graph availability.

use std::sync::Arc;

use daimon_graph::{BlastRadiusEntry, GraphClient, TargetRef as GraphTargetRef};
use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

use crate::approvals::ApprovalRecord;

/// Maximum hops to traverse from the target. 4 covers PlanStep ↔ Plan ↔
/// Capability ↔ depends-on chain without exploding into the entire graph.
pub const DEFAULT_BLAST_RADIUS_DEPTH: u32 = 4;

/// One approval + its blast-radius summary, ready for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalWithBlastRadius {
    pub approval: ApprovalRecord,
    pub blast_radius: Vec<BlastRadiusEntry>,
}

/// Enrich a list of approvals with per-row blast-radius summaries.
/// Each approval whose `target_ref` is `None` gets an empty list. Graph
/// failures are logged + the approval keeps an empty list — never panic
/// or fail the batch.
pub async fn enrich_with_blast_radius(
    approvals: Vec<ApprovalRecord>,
    graph: &Arc<dyn GraphClient>,
    max_depth: u32,
) -> Vec<ApprovalWithBlastRadius> {
    let mut out = Vec::with_capacity(approvals.len());
    for approval in approvals {
        let blast_radius = match approval.target_ref.as_ref() {
            Some(tref) => blast_radius_for_target(graph, approval.tenant_id, tref, max_depth).await,
            None => Vec::new(),
        };
        out.push(ApprovalWithBlastRadius { approval, blast_radius });
    }
    out
}

/// Single-target blast-radius lookup. Returns `Vec::new()` on any graph
/// failure; logs the cause at warn level so it's visible in the operator
/// console.
pub async fn blast_radius_for_target(
    graph: &Arc<dyn GraphClient>,
    tenant_id: Uuid,
    target_ref: &str,
    max_depth: u32,
) -> Vec<BlastRadiusEntry> {
    let tref = GraphTargetRef::from(target_ref);
    match graph.blast_radius(tenant_id, &tref, max_depth).await {
        Ok(entries) => entries,
        Err(e) => {
            warn!(
                tenant_id = %tenant_id,
                target = target_ref,
                error = %e,
                "blast_radius query failed; rendering approval without summary"
            );
            Vec::new()
        }
    }
}
