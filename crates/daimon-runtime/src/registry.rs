use std::collections::HashMap;
use std::sync::Arc;

use daimon_core::{AgentId, Capability, CoreError};
use semver::VersionReq;
use tokio::sync::RwLock;

/// What the registry stores about a single agent.
#[derive(Debug, Clone)]
pub struct RegistryEntry {
    pub agent_id: AgentId,
    pub capabilities: Vec<Capability>,
}

/// Tracks which agents implement which capabilities, version-aware (D17).
///
/// Cheap reads, occasional writes (on agent register / unregister). Backed by
/// an `RwLock<HashMap>` — fine for Phase 1; if lookup becomes a hotspot, swap
/// in `dashmap` or a custom index.
#[derive(Clone, Default)]
pub struct CapabilityRegistry {
    inner: Arc<RwLock<HashMap<AgentId, RegistryEntry>>>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an agent with its capabilities. Replaces any prior entry.
    pub async fn register(&self, agent_id: AgentId, capabilities: Vec<Capability>) {
        let mut guard = self.inner.write().await;
        guard.insert(
            agent_id.clone(),
            RegistryEntry {
                agent_id,
                capabilities,
            },
        );
    }

    /// Remove an agent.
    pub async fn unregister(&self, agent_id: &AgentId) {
        let mut guard = self.inner.write().await;
        guard.remove(agent_id);
    }

    /// All registered agents.
    pub async fn all(&self) -> Vec<RegistryEntry> {
        let guard = self.inner.read().await;
        guard.values().cloned().collect()
    }

    /// Look up an entry by agent id.
    pub async fn get(&self, agent_id: &AgentId) -> Option<RegistryEntry> {
        let guard = self.inner.read().await;
        guard.get(agent_id).cloned()
    }

    /// Find agents that satisfy a capability name + version requirement (D17).
    ///
    /// Returns all matching agents; the bus chooses one when routing
    /// `Recipient::ByCapability`. Phase 1 has no load-balancing strategy.
    pub async fn find_by_capability(
        &self,
        name: &str,
        version_req: &VersionReq,
    ) -> Vec<RegistryEntry> {
        let guard = self.inner.read().await;
        guard
            .values()
            .filter(|entry| {
                entry
                    .capabilities
                    .iter()
                    .any(|cap| cap.name == name && version_req.matches(&cap.version))
            })
            .cloned()
            .collect()
    }

    /// Find a single agent matching the capability — convenience wrapper.
    /// Returns `CapabilityNotFound` if no agent matches.
    pub async fn require_by_capability(
        &self,
        name: &str,
        version_req: &VersionReq,
    ) -> Result<RegistryEntry, CoreError> {
        self.find_by_capability(name, version_req)
            .await
            .into_iter()
            .next()
            .ok_or_else(|| CoreError::CapabilityNotFound {
                name: name.to_owned(),
                version_req: version_req.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daimon_core::Capability;
    use semver::Version;

    fn echo_cap_v1() -> Capability {
        Capability::read_only("test.echo", Version::new(1, 0, 0))
    }

    fn echo_cap_v2() -> Capability {
        Capability::read_only("test.echo", Version::new(2, 0, 0))
    }

    #[tokio::test]
    async fn register_then_lookup_by_id() {
        let reg = CapabilityRegistry::new();
        reg.register(AgentId::new("alpha"), vec![echo_cap_v1()]).await;
        let entry = reg.get(&AgentId::new("alpha")).await.unwrap();
        assert_eq!(entry.capabilities.len(), 1);
    }

    #[tokio::test]
    async fn version_req_matches_compatible_only() {
        let reg = CapabilityRegistry::new();
        reg.register(AgentId::new("v1"), vec![echo_cap_v1()]).await;
        reg.register(AgentId::new("v2"), vec![echo_cap_v2()]).await;

        let req_v1: VersionReq = "^1".parse().unwrap();
        let matches_v1 = reg.find_by_capability("test.echo", &req_v1).await;
        assert_eq!(matches_v1.len(), 1);
        assert_eq!(matches_v1[0].agent_id.as_str(), "v1");

        let req_any: VersionReq = "*".parse().unwrap();
        let matches_any = reg.find_by_capability("test.echo", &req_any).await;
        assert_eq!(matches_any.len(), 2);
    }

    #[tokio::test]
    async fn require_returns_not_found_for_missing_capability() {
        let reg = CapabilityRegistry::new();
        let req: VersionReq = "^1".parse().unwrap();
        let err = reg
            .require_by_capability("missing.cap", &req)
            .await
            .unwrap_err();
        match err {
            CoreError::CapabilityNotFound { name, .. } => assert_eq!(name, "missing.cap"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn unregister_removes_entry() {
        let reg = CapabilityRegistry::new();
        reg.register(AgentId::new("alpha"), vec![echo_cap_v1()]).await;
        reg.unregister(&AgentId::new("alpha")).await;
        assert!(reg.get(&AgentId::new("alpha")).await.is_none());
    }
}
