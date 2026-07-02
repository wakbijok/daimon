//! Phase 8 — graph-tier server-fns. Single endpoint right now: a
//! blast-radius lookup for a given `target_ref`. The future
//! `/admin/approvals` page consumes this to render the impact summary
//! per pending action.
//!
//! Returns an empty list if no graph tier is configured (`DAIMON_GRAPH_URL`
//! unset at boot) so the UI can degrade gracefully.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlastRadiusRow {
    pub kind: String,
    pub id: String,
    pub label: String,
    pub depth: u32,
}

#[server]
pub async fn lookup_blast_radius(
    target_ref: String,
    max_depth: Option<u32>,
) -> Result<Vec<BlastRadiusRow>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;
    use daimon_graph::TargetRef as GraphTargetRef;

    let _claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let depth = max_depth.unwrap_or(daimon_guard::DEFAULT_BLAST_RADIUS_DEPTH);

    let Some(graph) = state.graph.as_ref() else {
        return Ok(Vec::new());
    };

    let entries = graph
        .blast_radius(&GraphTargetRef::from(target_ref.as_str()), depth)
        .await
        .map_err(|e| ServerFnError::new(format!("blast_radius: {e}")))?;

    Ok(entries
        .into_iter()
        .map(|e| BlastRadiusRow {
            kind: format!("{:?}", e.kind),
            id: e.id,
            label: e.label,
            depth: e.depth,
        })
        .collect())
}
