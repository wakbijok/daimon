use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::refspec::TargetRef;

/// What kind of managed target this is. Used by broker for transport dispatch
/// and by inventory queries for filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// A virtualization / orchestration platform (PVE, K8s API server, etc.)
    Platform,
    /// A network device (router, switch, firewall, AP)
    Network,
    /// A Linux/Unix or Windows host for shell exec
    Host,
    /// An application accessed over its REST API
    App,
}

/// Which transport speaks to this target. Phase 2 supports SSH + REST + SNMP;
/// gRPC slot exists but is unimplemented until needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Ssh,
    Rest,
    Snmp,
    Grpc,
}

/// Metadata about a managed target that worker agents may see.
///
/// **Crucially does NOT carry a credential ref.** Host and port ARE visible
/// to agents — they're needed to compose multi-target operations (e.g.
/// "block traffic from host A to host B"), and they're not secret on their
/// own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetMetadata {
    pub r#ref: TargetRef,
    pub kind: TargetKind,
    pub transport: TransportKind,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// Capability names (semver-agnostic) this target supports — informational
    /// only; runtime capability discovery uses the agent registry.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// Full managed-target record. **Internal — broker only.**
///
/// Workers receive `TargetMetadata` via inventory queries. Only the broker
/// reads `ManagedTarget` and extracts `credential_ref` to resolve via vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedTarget {
    pub r#ref: TargetRef,
    pub kind: TargetKind,
    pub transport: TransportKind,
    pub host: String,
    pub port: u16,
    /// Vaultwarden reference (`vault://...`) for this target's credential.
    /// **Never exposed to workers — D19/D21.**
    pub credential_ref: String,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl ManagedTarget {
    /// Strip the credential ref to produce the agent-visible metadata view.
    pub fn metadata(&self) -> TargetMetadata {
        TargetMetadata {
            r#ref: self.r#ref.clone(),
            kind: self.kind,
            transport: self.transport,
            host: self.host.clone(),
            port: self.port,
            labels: self.labels.clone(),
            capabilities: self.capabilities.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_managed() -> ManagedTarget {
        ManagedTarget {
            r#ref: TargetRef::parse("target://mikrotik-edge").unwrap(),
            kind: TargetKind::Network,
            transport: TransportKind::Ssh,
            host: "10.100.10.1".into(),
            port: 22,
            credential_ref: "vault://infra/network/mikrotik-edge".into(),
            labels: BTreeMap::new(),
            capabilities: vec!["network.firewall.address_list.add".into()],
        }
    }

    #[test]
    fn metadata_view_drops_credential_ref() {
        let mt = sample_managed();
        let md = mt.metadata();
        let json = serde_json::to_string(&md).unwrap();
        assert!(!json.contains("credential_ref"));
        assert!(!json.contains("vault://"));
        // but the operational data is preserved
        assert!(json.contains("10.100.10.1"));
        assert!(json.contains("network.firewall.address_list.add"));
    }

    #[test]
    fn managed_target_serde_round_trip() {
        let mt = sample_managed();
        let json = serde_json::to_string(&mt).unwrap();
        let back: ManagedTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(back, mt);
    }
}
