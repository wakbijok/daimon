//! Graph-side type primitives.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// `target://` reference — the same shape `daimon-inventory` uses.
/// Kept as a thin wrapper here so the graph crate stays
/// dependency-free against `daimon-inventory`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TargetRef(pub String);

impl TargetRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for TargetRef {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for TargetRef {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeKind {
    Tenant,
    Agent,
    Capability,
    Target,
    Credential,
    Plan,
    PlanStep,
    AuditEvent,
    User,
}

impl NodeKind {
    pub fn label(&self) -> &'static str {
        match self {
            NodeKind::Tenant => "Tenant",
            NodeKind::Agent => "Agent",
            NodeKind::Capability => "Capability",
            NodeKind::Target => "Target",
            NodeKind::Credential => "Credential",
            NodeKind::Plan => "Plan",
            NodeKind::PlanStep => "PlanStep",
            NodeKind::AuditEvent => "AuditEvent",
            NodeKind::User => "User",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelKind {
    Owns,
    ExecutesAs,
    ProvidesCapability,
    DependsOnTarget,
    RequiresCredential,
    EmittedBy,
    StepOf,
    BlastRadius,
}

impl RelKind {
    pub fn label(&self) -> &'static str {
        match self {
            RelKind::Owns => "OWNS",
            RelKind::ExecutesAs => "EXECUTES_AS",
            RelKind::ProvidesCapability => "PROVIDES_CAPABILITY",
            RelKind::DependsOnTarget => "DEPENDS_ON_TARGET",
            RelKind::RequiresCredential => "REQUIRES_CREDENTIAL",
            RelKind::EmittedBy => "EMITTED_BY",
            RelKind::StepOf => "STEP_OF",
            RelKind::BlastRadius => "BLAST_RADIUS",
        }
    }
}

/// A node returned from a blast-radius query, in dependency-distance order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlastRadiusEntry {
    pub kind: NodeKind,
    pub id: String,
    pub label: String,
    pub depth: u32,
}

/// Plan persistence DTO — shape mirrors `daimon-orchestrator::Plan` but
/// kept local so this crate doesn't depend on the orchestrator crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPlan {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub intent: String,
    pub created_at: DateTime<Utc>,
    pub steps: Vec<GraphPlanStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPlanStep {
    pub id: Uuid,
    pub capability_name: String,
    pub capability_version: String,
    pub target_ref: TargetRef,
    pub depends_on: Vec<Uuid>,
}
