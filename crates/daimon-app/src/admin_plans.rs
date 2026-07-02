//! Phase 6 D1 — server-fns backing `/admin/plans`.
//!
//! Exposes the orchestrator's plan CRUD + run path through `require_admin()`.
//! Phase 6 D2 adds `create_plan_from_intent` which prompts the LLM.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRow {
    pub id: String,
    pub intent: String,
    pub status: String,
    pub created_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRow {
    pub id: String,
    pub step_index: i32,
    pub capability_name: String,
    pub target_ref: Option<String>,
    pub depends_on: Vec<String>,
    pub status: String,
    pub result_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDefDto {
    pub capability_name: String,
    pub capability_version: String,
    #[serde(default)]
    pub target_ref: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    pub depends_on_index: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePlanRequest {
    pub intent: String,
    pub steps: Vec<StepDefDto>,
}

#[server]
pub async fn list_plans() -> Result<Vec<PlanRow>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let _claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let plans = state
        .orchestrator
        .list_plans(100)
        .await
        .map_err(|e| ServerFnError::new(format!("list_plans: {e}")))?;
    Ok(plans
        .into_iter()
        .map(|p| PlanRow {
            id: p.id.to_string(),
            intent: p.intent,
            status: format!("{:?}", p.status).to_lowercase(),
            created_at: p.created_at.to_rfc3339(),
            finished_at: p.finished_at.map(|d| d.to_rfc3339()),
        })
        .collect())
}

#[server]
pub async fn get_plan_steps(plan_id: String) -> Result<Vec<StepRow>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let _claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let plan_id = uuid::Uuid::parse_str(&plan_id)
        .map_err(|e| ServerFnError::new(format!("parse plan_id: {e}")))?;
    let steps = state
        .orchestrator
        .list_steps(plan_id)
        .await
        .map_err(|e| ServerFnError::new(format!("list_steps: {e}")))?;
    Ok(steps
        .into_iter()
        .map(|s| StepRow {
            id: s.id.to_string(),
            step_index: s.step_index,
            capability_name: s.capability_name,
            target_ref: s.target_ref,
            depends_on: s.depends_on.iter().map(|u| u.to_string()).collect(),
            status: format!("{:?}", s.status).to_lowercase(),
            result_summary: s
                .result
                .as_ref()
                .map(|v| {
                    let s = v.to_string();
                    if s.len() > 200 { format!("{}…", &s[..200]) } else { s }
                }),
        })
        .collect())
}

#[server]
pub async fn create_plan(req: CreatePlanRequest) -> Result<String, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;
    use daimon_orchestrator::StepDef;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let steps: Vec<StepDef> = req
        .steps
        .into_iter()
        .map(|d| StepDef {
            capability_name: d.capability_name,
            capability_version: d.capability_version,
            target_ref: d.target_ref,
            credential_ref: d.credential_ref,
            params: d.params,
            depends_on_index: d.depends_on_index,
        })
        .collect();
    let plan = state
        .orchestrator
        .create_plan(Some(claims.user_id), &req.intent, steps)
        .await
        .map_err(|e| ServerFnError::new(format!("create_plan: {e}")))?;
    Ok(plan.id.to_string())
}

#[server]
pub async fn run_plan(plan_id: String) -> Result<String, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let id = uuid::Uuid::parse_str(&plan_id)
        .map_err(|e| ServerFnError::new(format!("parse plan_id: {e}")))?;
    let final_status = state
        .orchestrator
        .run_plan(id, &claims.sub)
        .await
        .map_err(|e| ServerFnError::new(format!("run_plan: {e}")))?;
    Ok(format!("{:?}", final_status).to_lowercase())
}

#[server]
pub async fn plan_from_intent(intent: String) -> Result<String, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;
    use daimon_llm::AnthropicClient;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let llm = AnthropicClient::from_env()
        .map_err(|e| ServerFnError::new(format!("llm init: {e}")))?;
    let catalog = capability_catalog(&state).await;
    let plan = state
        .orchestrator
        .plan_from_intent(Some(claims.user_id), &intent, &catalog, &llm)
        .await
        .map_err(|e| ServerFnError::new(format!("plan_from_intent: {e}")))?;
    Ok(plan.id.to_string())
}

/// Project the planner catalog from the LIVE capability registry (AC-P2-03).
/// Each registered driver capability is listed with its name/version/typed
/// params — matching the flipped plan-step contract (typed params, not a raw
/// command). Adding a new driver/connector makes it plannable with no edit here.
#[cfg(feature = "ssr")]
async fn capability_catalog(state: &crate::state::AppState) -> String {
    let mut caps = state.harness.capabilities().await;
    caps.sort_by(|a, b| a.name.cmp(&b.name).then(b.version.cmp(&a.version)));
    let mut out = String::from(
        "Capabilities (use capability_name EXACTLY; put the TYPED params under `params`):\n",
    );
    for c in caps {
        out.push_str(&format!(
            "- {} v{} — {}\n",
            c.name,
            c.version,
            c.description.as_deref().unwrap_or("(no description)")
        ));
        match c
            .schema
            .as_ref()
            .and_then(|s| s.get("properties"))
            .and_then(|p| p.as_object())
        {
            Some(props) if !props.is_empty() => {
                let names: Vec<&str> = props.keys().map(String::as_str).collect();
                out.push_str(&format!("  params: {}\n", names.join(", ")));
            }
            _ => out.push_str("  params: target_ref only (read capability)\n"),
        }
    }
    out.push_str("\nAvailable targets: any registered target_ref (e.g. target://mikrotik-edge).\n");
    out
}
