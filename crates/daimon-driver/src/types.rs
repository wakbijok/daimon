//! Typed structured returns for the four `Driver` verbs (FR-CON-06).
//!
//! Regexing raw stdout is what makes vendor logic rot. Each verb returns a
//! typed value so the orchestrator/LLM reasons over structure, not text:
//! `describe → TargetShape`, `read_state → StateDoc`, `diagnose → Vec<Finding>`,
//! `remediate → Receipt`.

use serde::{Deserialize, Serialize};

use crate::TargetClass;

/// What a target IS, as the driver understands it (`describe`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetShape {
    /// The semantic class this target belongs to.
    pub class: TargetClass,
    /// Driver-shaped identity block (name, model, version, node id, …).
    pub identity: serde_json::Value,
    /// Human-readable one-line summary.
    pub summary: String,
}

/// A live typed snapshot for a selector (`read_state`). The `doc` is parsed —
/// never raw stdout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDoc {
    /// The target ref (or selector) this snapshot describes.
    pub target: String,
    /// The parsed, typed snapshot.
    pub doc: serde_json::Value,
}

/// Severity of a diagnostic [`Finding`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Critical,
}

/// A diagnostic finding (`diagnose`). Carries a suggested remediation — this is
/// the diagnose → plan bridge: the orchestrator lifts
/// `suggested_capability` + `suggested_params` straight into a plan step
/// (FR-CON-20/21).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub title: String,
    pub severity: Severity,
    pub detail: String,
    /// The capability that would remediate this finding, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_capability: Option<String>,
    /// Params to fill straight into the remediation step (JSON object).
    #[serde(default)]
    pub suggested_params: serde_json::Value,
}

/// The outcome of a write (`remediate`). `changed` is what saga rollback reads
/// to build the compensating call's params — targeting exactly what was
/// changed, not re-deriving from the original request (D18, FR-CON-22).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub capability: String,
    /// What was created/changed, plus prior values the inverse needs (e.g.
    /// removed rule id, pre-patch resource limits).
    pub changed: serde_json::Value,
}
