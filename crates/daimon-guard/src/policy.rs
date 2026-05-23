//! Policy DSL — Rust + TOML, type-checked at load.
//!
//! TOML shape:
//!
//! ```toml
//! [[rule]]
//! capability = "network.routeros.*"            # glob — required
//! decision = "allow"                           # allow | deny | require_approval
//! description = "All RouterOS reads"           # optional
//!
//! [[rule]]
//! capability = "network.firewall.*"
//! decision = "require_approval"
//! description = "All firewall writes need explicit operator approval"
//!
//! [[rule]]
//! capability = "vault.*"
//! decision = "deny"
//! description = "Vault writes never go through orchestrator paths"
//! ```
//!
//! Rules are evaluated in order; the FIRST matching rule wins. If no rule
//! matches, the default is `deny` (fail-closed). Read-only capabilities
//! (per Capability metadata) bypass policy and always allow.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny,
    RequireApproval,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Glob pattern matched against capability name. Use `*` for prefix
    /// match, e.g. `"network.firewall.*"`.
    pub capability: String,
    pub decision: Decision,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicyConfig {
    #[serde(default, rename = "rule")]
    pub rules: Vec<PolicyRule>,
}

#[derive(Debug, Clone, Copy)]
pub struct PolicyVerdict {
    pub decision: Decision,
    pub rule_index: Option<usize>,
}

/// The engine — owns the loaded ruleset.
#[derive(Debug, Clone)]
pub struct PolicyEngine {
    rules: Vec<PolicyRule>,
    default_decision: Decision,
}

impl PolicyEngine {
    pub fn new(rules: Vec<PolicyRule>) -> Self {
        Self {
            rules,
            default_decision: Decision::Deny,
        }
    }

    /// Load from a TOML file. Returns an empty (default-deny) engine if the
    /// file is missing.
    pub fn from_toml_file(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::new(Vec::new()));
        }
        let text = std::fs::read_to_string(path)?;
        let cfg: PolicyConfig = toml::from_str(&text)?;
        Ok(Self::new(cfg.rules))
    }

    pub fn from_toml_str(s: &str) -> Result<Self> {
        let cfg: PolicyConfig = toml::from_str(s).map_err(Error::Toml)?;
        Ok(Self::new(cfg.rules))
    }

    pub fn rules(&self) -> &[PolicyRule] {
        &self.rules
    }

    pub fn default_decision(&self) -> Decision {
        self.default_decision
    }

    pub fn evaluate(&self, capability: &str) -> PolicyVerdict {
        for (i, rule) in self.rules.iter().enumerate() {
            if glob_match(&rule.capability, capability) {
                return PolicyVerdict {
                    decision: rule.decision,
                    rule_index: Some(i),
                };
            }
        }
        PolicyVerdict {
            decision: self.default_decision,
            rule_index: None,
        }
    }
}

/// Minimal glob: supports trailing `*` (prefix match) and exact match.
fn glob_match(pattern: &str, value: &str) -> bool {
    if pattern == value {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return value == prefix || value.starts_with(&format!("{prefix}."));
    }
    if pattern == "*" {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_prefix_match() {
        assert!(glob_match("network.routeros.*", "network.routeros.system_info"));
        assert!(glob_match("network.routeros.*", "network.routeros"));
        assert!(!glob_match("network.routeros.*", "network.firewall.add"));
    }

    #[test]
    fn first_matching_rule_wins() {
        let engine = PolicyEngine::from_toml_str(
            r#"
            [[rule]]
            capability = "network.routeros.*"
            decision = "allow"

            [[rule]]
            capability = "network.*"
            decision = "deny"
            "#,
        )
        .unwrap();
        let v = engine.evaluate("network.routeros.system_info");
        assert_eq!(v.decision, Decision::Allow);
        let v = engine.evaluate("network.firewall.add");
        assert_eq!(v.decision, Decision::Deny);
    }

    #[test]
    fn no_match_defaults_to_deny() {
        let engine = PolicyEngine::new(Vec::new());
        let v = engine.evaluate("anything.goes");
        assert_eq!(v.decision, Decision::Deny);
    }
}
