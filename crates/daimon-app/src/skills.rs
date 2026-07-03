//! Skills as workflow-templates (P5-5).
//!
//! A **skill** is a named, parameterised **plan template** — NOT a new dispatch
//! primitive. Running a skill validates its params, substitutes them into a set
//! of orchestrator `StepDef`s, and hands them to `create_plan` + `run_plan`, so a
//! skill reuses the EXACT plan/saga/approval/audit engine a chat- or admin-
//! authored plan uses. There is no second executor — the same reason a gateway
//! is just another `ReplySink`. (SDS §skills decision: workflow-templates.)
//!
//! Definitions live in `deploy/skills/*.toml` (`DAIMON_SKILLS_DIR`), loaded once
//! at boot into a [`SkillLibrary`] held in `AppState`. Every `{param}` slot in a
//! step's `target` / `params` is filled ONLY from a value that first passed
//! `param::validate` against its declared class — the same injection chokepoint
//! the connector uses, so a skill cannot smuggle a shell metacharacter or a
//! path-traversal into a step.

#![cfg(feature = "ssr")]

use std::collections::BTreeMap;
use std::path::Path;

use daimon_driver::{ParamClass, param};
use daimon_orchestrator::StepDef;
use serde::Deserialize;
use serde_json::Value as Json;

#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("unknown skill `{0}`")]
    Unknown(String),
    #[error("missing required param `{0}`")]
    MissingParam(String),
    #[error("param `{param}` invalid: {source}")]
    Param {
        param: String,
        source: param::ParamError,
    },
    #[error("io reading skills dir `{path}`: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("parse skill `{path}`: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
    #[error("skill `{skill}` step {step} references undeclared param `{param}`")]
    UndeclaredSlot {
        skill: String,
        step: usize,
        param: String,
    },
}

/// One skill = a parameterised plan template.
#[derive(Debug, Clone, Deserialize)]
pub struct Skill {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "param")]
    pub params: Vec<SkillParam>,
    #[serde(default, rename = "step")]
    pub steps: Vec<SkillStep>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SkillParam {
    pub name: String,
    /// Param class: `safe_text` | `identifier` | `int` | `cidr` (default safe_text).
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SkillStep {
    pub capability: String,
    pub version: String,
    /// Target-ref template (may carry `{param}` slots).
    #[serde(default)]
    pub target: Option<String>,
    /// Params template — string leaves may carry `{param}` slots.
    #[serde(default)]
    pub params: Json,
    /// Zero-based indices of earlier steps this one depends on.
    #[serde(default)]
    pub depends_on: Vec<usize>,
}

fn parse_class(s: Option<&str>) -> ParamClass {
    match s {
        Some("cidr") => ParamClass::Cidr,
        Some("identifier") => ParamClass::Identifier,
        Some("int") => ParamClass::Int,
        _ => ParamClass::SafeText,
    }
}

impl Skill {
    /// Validate + substitute the operator's inputs into concrete `StepDef`s. Every
    /// declared param must be supplied AND pass `param::validate` for its class
    /// (the injection chokepoint). Returns the plan's step defs, ready for
    /// `create_plan`.
    pub fn instantiate(&self, inputs: &BTreeMap<String, String>) -> Result<Vec<StepDef>, SkillError> {
        // 1. Validate every declared param up front.
        let mut validated: BTreeMap<String, String> = BTreeMap::new();
        for p in &self.params {
            let raw = inputs
                .get(&p.name)
                .ok_or_else(|| SkillError::MissingParam(p.name.clone()))?;
            let class = parse_class(p.class.as_deref());
            param::validate(raw, &class).map_err(|source| SkillError::Param {
                param: p.name.clone(),
                source,
            })?;
            validated.insert(p.name.clone(), raw.clone());
        }

        // 2. Substitute into each step. A `{slot}` with no declared+validated
        // param is an error — a template can never emit an un-filled slot.
        let mut defs = Vec::with_capacity(self.steps.len());
        for (i, step) in self.steps.iter().enumerate() {
            let target_ref = match &step.target {
                Some(t) => Some(substitute(t, &self.name, i, &validated)?),
                None => None,
            };
            let params = substitute_json(&step.params, &self.name, i, &validated)?;
            defs.push(StepDef {
                capability_name: step.capability.clone(),
                capability_version: step.version.clone(),
                target_ref,
                credential_ref: None,
                params,
                depends_on_index: step.depends_on.clone(),
            });
        }
        Ok(defs)
    }
}

/// Replace every `{name}` in `template` with `validated[name]`. An unresolved
/// slot is an error (mirrors the connector's `substitute` chokepoint).
fn substitute(
    template: &str,
    skill: &str,
    step: usize,
    validated: &BTreeMap<String, String>,
) -> Result<String, SkillError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or_else(|| SkillError::UndeclaredSlot {
            skill: skill.to_string(),
            step,
            param: after.to_string(),
        })?;
        let name = &after[..close];
        let val = validated.get(name).ok_or_else(|| SkillError::UndeclaredSlot {
            skill: skill.to_string(),
            step,
            param: name.to_string(),
        })?;
        out.push_str(val);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Recursively substitute `{param}` in every string leaf of a JSON template.
fn substitute_json(
    v: &Json,
    skill: &str,
    step: usize,
    validated: &BTreeMap<String, String>,
) -> Result<Json, SkillError> {
    match v {
        Json::String(s) => Ok(Json::String(substitute(s, skill, step, validated)?)),
        Json::Array(a) => Ok(Json::Array(
            a.iter()
                .map(|x| substitute_json(x, skill, step, validated))
                .collect::<Result<_, _>>()?,
        )),
        Json::Object(o) => {
            let mut m = serde_json::Map::new();
            for (k, val) in o {
                m.insert(k.clone(), substitute_json(val, skill, step, validated)?);
            }
            Ok(Json::Object(m))
        }
        other => Ok(other.clone()),
    }
}

/// The loaded skills, indexed by name.
#[derive(Debug, Clone, Default)]
pub struct SkillLibrary {
    skills: BTreeMap<String, Skill>,
}

impl SkillLibrary {
    /// Load every `*.toml` skill from `dir`. Absent/empty dir → empty library.
    pub fn from_dir(dir: &Path) -> Result<Self, SkillError> {
        let mut skills = BTreeMap::new();
        if !dir.exists() {
            return Ok(Self { skills });
        }
        let entries = std::fs::read_dir(dir).map_err(|e| SkillError::Io {
            path: dir.display().to_string(),
            source: e,
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| SkillError::Io {
                path: dir.display().to_string(),
                source: e,
            })?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }
            let text = std::fs::read_to_string(&path).map_err(|e| SkillError::Io {
                path: path.display().to_string(),
                source: e,
            })?;
            let skill: Skill = toml::from_str(&text).map_err(|e| SkillError::Parse {
                path: path.display().to_string(),
                source: e,
            })?;
            skills.insert(skill.name.clone(), skill);
        }
        Ok(Self { skills })
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    /// Names + descriptions for the run surface.
    pub fn list(&self) -> Vec<(String, String)> {
        self.skills
            .values()
            .map(|s| (s.name.clone(), s.description.clone()))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKILL_TOML: &str = r#"
name = "restart-k8s-workload"
description = "Roll-restart a deployment then verify."
[[param]]
name = "target"
class = "safe_text"
[[param]]
name = "namespace"
class = "identifier"
[[param]]
name = "deployment"
class = "identifier"
[[step]]
capability = "orchestrator.k8s.deploy.restart"
version = "1.0.0"
target = "{target}"
params = { namespace = "{namespace}", name = "{deployment}" }
[[step]]
capability = "orchestrator.k8s.pod.status"
version = "1.0.0"
target = "{target}"
params = { namespace = "{namespace}", name = "{deployment}" }
depends_on = [0]
"#;

    fn skill() -> Skill {
        toml::from_str(SKILL_TOML).unwrap()
    }

    #[test]
    fn instantiate_substitutes_into_stepdefs() {
        let mut inputs = BTreeMap::new();
        inputs.insert("target".to_string(), "target://k3s-lab".to_string());
        inputs.insert("namespace".to_string(), "web".to_string());
        inputs.insert("deployment".to_string(), "nginx".to_string());
        let defs = skill().instantiate(&inputs).unwrap();
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].capability_name, "orchestrator.k8s.deploy.restart");
        assert_eq!(defs[0].target_ref.as_deref(), Some("target://k3s-lab"));
        assert_eq!(defs[0].params["namespace"], "web");
        assert_eq!(defs[0].params["name"], "nginx");
        assert_eq!(defs[1].depends_on_index, vec![0]);
    }

    #[test]
    fn missing_param_rejected() {
        let mut inputs = BTreeMap::new();
        inputs.insert("target".to_string(), "target://x".to_string());
        // namespace + deployment missing
        assert!(matches!(
            skill().instantiate(&inputs).unwrap_err(),
            SkillError::MissingParam(_)
        ));
    }

    #[test]
    fn injection_param_rejected_at_validate() {
        // `namespace` is `identifier` — a path-traversal / metachar value is
        // rejected before any StepDef is built (the injection chokepoint).
        let mut inputs = BTreeMap::new();
        inputs.insert("target".to_string(), "target://x".to_string());
        inputs.insert("namespace".to_string(), "web/../../secret".to_string());
        inputs.insert("deployment".to_string(), "nginx".to_string());
        assert!(matches!(
            skill().instantiate(&inputs).unwrap_err(),
            SkillError::Param { .. }
        ));
    }

    #[test]
    fn library_loads_from_str_roundtrip() {
        let s = skill();
        let mut lib = SkillLibrary::default();
        lib.skills.insert(s.name.clone(), s);
        assert_eq!(lib.len(), 1);
        assert!(lib.get("restart-k8s-workload").is_some());
        assert_eq!(lib.list()[0].0, "restart-k8s-workload");
    }
}
