//! Server fns for skills (workflow-templates, P5-5). `list_skills` is available
//! to any authenticated operator; `run_skill` instantiates the skill's plan
//! template and runs it through the orchestrator (saga/approval/audit) — it is
//! `admin`-gated, and any WRITE step is additionally approval-gated by the Guard.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SkillDto {
    pub name: String,
    pub description: String,
}

#[server]
pub async fn list_skills() -> Result<Vec<SkillDto>, ServerFnError> {
    use crate::auth_guard::require_authenticated;
    use crate::state::AppState;

    require_authenticated().await?;
    let state = expect_context::<AppState>();
    Ok(state
        .skills
        .list()
        .into_iter()
        .map(|(name, description)| SkillDto { name, description })
        .collect())
}

/// Instantiate + run a skill. Returns the created plan id; the run proceeds in
/// the background (approval-gated writes wait in the approvals inbox), so the
/// caller does not block on a plan that needs a human decision.
#[server]
pub async fn run_skill(
    name: String,
    params: BTreeMap<String, String>,
) -> Result<String, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();

    let skill = state
        .skills
        .get(&name)
        .ok_or_else(|| ServerFnError::new(format!("unknown skill: {name}")))?
        .clone();

    // Validate + substitute -> concrete StepDefs (the injection chokepoint runs
    // inside instantiate). A bad/missing param fails HERE, before any plan row.
    let steps = skill
        .instantiate(&params)
        .map_err(|e| ServerFnError::new(format!("skill `{name}`: {e}")))?;

    let intent = format!("skill:{name}");
    let plan = state
        .orchestrator
        .create_plan(Some(claims.user_id), &intent, steps)
        .await
        .map_err(|e| ServerFnError::new(format!("create_plan: {e}")))?;

    // Run in the background — run_plan awaits approvals inline, so a skill with an
    // approval-gated write would otherwise hang this request.
    let plan_id = plan.id;
    let orch = state.orchestrator.clone();
    let actor = claims.sub.clone();
    tokio::spawn(async move {
        if let Err(e) = orch.run_plan(plan_id, &actor).await {
            tracing::warn!(plan = %plan_id, error = %e, "skill plan run failed");
        }
    });

    Ok(plan_id.to_string())
}
