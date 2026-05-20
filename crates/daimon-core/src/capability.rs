use semver::Version;
use serde::{Deserialize, Serialize};

/// A capability an agent implements (D17 — versioned).
///
/// `name` is the dotted-path capability identifier — e.g.
/// `"network.firewall.address_list.add"`, `"platform.workload.list"`.
/// `version` is the semver of the capability's contract; orchestrator plans
/// pin against `semver::VersionReq` for backward-compat windows.
/// `compensating` (D18) names the inverse capability used for saga rollback,
/// e.g. `address_list.add` ↔ `address_list.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub name: String,
    pub version: Version,
    #[serde(default)]
    pub description: Option<String>,
    /// JSON Schema for the capability's parameters. Optional in Phase 1;
    /// the orchestrator will require it before generating plans.
    #[serde(default)]
    pub schema: Option<serde_json::Value>,
    /// Capability name of the compensating action used for saga rollback.
    /// `None` means the capability is read-only or inherently irreversible.
    #[serde(default)]
    pub compensating: Option<CompensatingCapability>,
    /// If true, the capability cannot be rolled back automatically — orchestrator
    /// must obtain explicit user acknowledgement before invoking it.
    #[serde(default)]
    pub irreversible: bool,
}

impl Capability {
    /// Convenience constructor for a read-only capability (no compensation needed).
    pub fn read_only(name: impl Into<String>, version: Version) -> Self {
        Self {
            name: name.into(),
            version,
            description: None,
            schema: None,
            compensating: None,
            irreversible: false,
        }
    }
}

/// Reference to the compensating capability for a write capability (D18).
///
/// Names the inverse-action capability — the orchestrator will look it up in
/// the registry and dispatch with rollback parameters from the original step's
/// receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompensatingCapability {
    pub name: String,
    /// Version requirement for the compensating capability. If `None`, the
    /// orchestrator picks the highest available version at rollback time.
    #[serde(default)]
    pub version_req: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_capability_has_no_compensation() {
        let cap = Capability::read_only("platform.workload.list", Version::new(1, 0, 0));
        assert!(cap.compensating.is_none());
        assert!(!cap.irreversible);
    }

    #[test]
    fn write_capability_round_trips_via_serde() {
        let cap = Capability {
            name: "network.firewall.address_list.add".into(),
            version: Version::new(1, 0, 0),
            description: Some("Add an IP to a named address-list".into()),
            schema: None,
            compensating: Some(CompensatingCapability {
                name: "network.firewall.address_list.remove".into(),
                version_req: Some("^1".into()),
            }),
            irreversible: false,
        };
        let json = serde_json::to_string(&cap).unwrap();
        let back: Capability = serde_json::from_str(&json).unwrap();
        assert_eq!(cap, back);
    }
}
