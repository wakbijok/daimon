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

    /// SERVER-SIDE read-only disposition — the sole authority for whether a
    /// capability skips policy + approval (NFR-SAF-02). The broker derives the
    /// guard's read-only bit from THIS, never from a caller/LLM-supplied flag
    /// (which closes the H6/H7 bypass). A capability is read-only iff it has no
    /// compensating action, is not irreversible, AND its name carries a read
    /// verb. The name check is a fail-closed backstop: a write capability
    /// authored without a compensator and not marked irreversible (the same
    /// shape `read_only()` produces) would otherwise misclassify as read; the
    /// verb allowlist forces anything not clearly a read to be treated as a
    /// write. P2's typed connector verbs (describe/read_state/diagnose vs
    /// remediate) supersede the name heuristic.
    pub fn is_read(&self) -> bool {
        self.compensating.is_none() && !self.irreversible && Self::name_is_read(&self.name)
    }

    /// Read-verb allowlist over the dotted capability name. Anything not
    /// matched is treated as a write (fail-closed). Covers the RouterOS reads
    /// (`system_info`, `interface_list`, `ip_addresses`, `firewall_filter_list`)
    /// and the forthcoming connector read verbs.
    fn name_is_read(name: &str) -> bool {
        const READ_MARKERS: &[&str] = &[
            "_info", "_list", "list_", ".list", "_addresses", "ip_addresses",
            "_status", ".status", ".read", "_read", "read_state", "describe",
            "diagnose", ".get", "_get", "_show", "_print",
        ];
        READ_MARKERS.iter().any(|m| name.contains(m))
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
    fn is_read_classifies_the_routeros_capabilities() {
        let v = Version::new(1, 0, 0);
        // The four reads (built via read_only) — must classify as read.
        for name in [
            "network.routeros.system_info",
            "network.routeros.interface_list",
            "network.routeros.ip_addresses",
            "network.routeros.firewall_filter_list",
        ] {
            assert!(Capability::read_only(name, v.clone()).is_read(), "{name} should be read");
        }
        // Write with a compensator — not read.
        let add = Capability {
            name: "network.routeros.firewall_add_drop_rule".into(),
            version: v.clone(),
            description: None,
            schema: None,
            compensating: Some(CompensatingCapability {
                name: "network.routeros.firewall_remove_rule".into(),
                version_req: None,
            }),
            irreversible: false,
        };
        assert!(!add.is_read(), "add_drop_rule is a write");
        // Irreversible write — not read.
        let remove = Capability {
            name: "network.routeros.firewall_remove_rule".into(),
            version: v.clone(),
            description: None,
            schema: None,
            compensating: None,
            irreversible: true,
        };
        assert!(!remove.is_read(), "remove_rule is an irreversible write");
        // Fail-closed: a write-shaped cap with no compensator/irreversible but a
        // non-read name is still treated as a write.
        let sneaky = Capability::read_only("network.routeros.reboot", v);
        assert!(!sneaky.is_read(), "reboot has no read verb -> treated as write");
    }

    #[test]
    fn dotted_connector_read_verbs_classify_as_read() {
        // P2 connector profiles use dotted verbs (e.g. `orchestrator.k8s.pod.status`).
        // `.status` is a read verb; the deploy write verbs are not.
        let v = Version::new(1, 0, 0);
        assert!(
            Capability::read_only("orchestrator.k8s.pod.status", v.clone()).is_read(),
            "pod.status must classify as a read"
        );
        // Writes with a dotted non-read verb stay writes (fail-closed).
        let restart = Capability {
            name: "orchestrator.k8s.deploy.restart".into(),
            version: v.clone(),
            description: None,
            schema: None,
            compensating: Some(CompensatingCapability {
                name: "orchestrator.k8s.deploy.rollback".into(),
                version_req: None,
            }),
            irreversible: false,
        };
        assert!(!restart.is_read(), "deploy.restart is a write");
        let rollback = Capability::read_only("orchestrator.k8s.deploy.rollback", v);
        assert!(!rollback.is_read(), "deploy.rollback has no read verb -> write");
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
