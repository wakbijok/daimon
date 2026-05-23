//! Plan + Step types — match the V006 schema shape.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Planning,
    AwaitingApproval,
    Executing,
    Succeeded,
    Failed,
    Cancelled,
    RolledBack,
}

impl PlanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Executing => "executing",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::RolledBack => "rolled_back",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "planning" => Self::Planning,
            "awaiting_approval" => Self::AwaitingApproval,
            "executing" => Self::Executing,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "rolled_back" => Self::RolledBack,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
    Compensated,
}

impl StepStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Compensated => "compensated",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "skipped" => Self::Skipped,
            "compensated" => Self::Compensated,
            _ => Self::Failed,
        }
    }
}

/// Hand-supplied step definition — what create_plan accepts. Step ids
/// (real Uuids) are assigned by the orchestrator at persist time;
/// `depends_on_index` refers to other items in the same definition list
/// by their `step_index` (zero-based).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDef {
    pub capability_name: String,
    pub capability_version: String,
    pub target_ref: Option<String>,
    pub credential_ref: Option<String>,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    pub depends_on_index: Vec<usize>,
}

/// Persisted plan row (from public.plans).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub created_by: Option<Uuid>,
    pub intent: String,
    pub status: PlanStatus,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Persisted step row (from public.plan_steps).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub step_index: i32,
    pub capability_name: String,
    pub capability_version: String,
    pub target_ref: Option<String>,
    pub credential_ref: Option<String>,
    pub params: serde_json::Value,
    pub depends_on: Vec<Uuid>,
    pub compensating_step_id: Option<Uuid>,
    pub status: StepStatus,
    pub result: Option<serde_json::Value>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}
