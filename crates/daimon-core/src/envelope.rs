use chrono::{DateTime, Utc};
use semver::VersionReq;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::id::AgentId;

/// An envelope is the unit of agent-to-agent communication.
///
/// `correlation_id` identifies a logical request across replies and delegated
/// sub-requests. `reply_to` lets the recipient send a response. `to` selects
/// the recipient either directly by agent id or indirectly by capability
/// (the runtime resolves capability addresses via the registry).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEnvelope {
    pub correlation_id: Uuid,
    pub from: AgentId,
    pub to: Recipient,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<AgentId>,
    pub body: serde_json::Value,
    pub audit: AuditMetadata,
}

impl AgentEnvelope {
    /// Create a new envelope with a fresh correlation id.
    pub fn new(from: AgentId, to: Recipient, body: serde_json::Value) -> Self {
        Self {
            correlation_id: Uuid::new_v4(),
            from,
            to,
            reply_to: None,
            body,
            audit: AuditMetadata::default(),
        }
    }

    /// Create a reply envelope that preserves the correlation id of the request.
    pub fn reply_to(request: &AgentEnvelope, from: AgentId, body: serde_json::Value) -> Self {
        Self {
            correlation_id: request.correlation_id,
            from,
            to: Recipient::Direct(request.from.clone()),
            reply_to: None,
            body,
            audit: AuditMetadata::default(),
        }
    }
}

/// How to address an envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Recipient {
    /// Send to a specific agent instance.
    Direct(AgentId),
    /// Send to any agent that implements a capability satisfying the version
    /// requirement. The runtime picks one (Phase 1: arbitrary; later: by load
    /// or by per-capability routing strategy).
    ByCapability {
        name: String,
        #[serde(with = "version_req_serde")]
        version_req: VersionReq,
    },
}

/// Metadata accompanying every envelope for audit / tracing purposes.
///
/// Phase 1 carries only the timestamp; Guard (Phase 5) will extend this with
/// actor, action description, target, decision evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditMetadata {
    pub ts: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_correlation_id: Option<Uuid>,
}

impl Default for AuditMetadata {
    fn default() -> Self {
        Self {
            ts: Utc::now(),
            parent_correlation_id: None,
        }
    }
}

mod version_req_serde {
    use semver::VersionReq;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &VersionReq, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<VersionReq, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_envelope_has_fresh_correlation_id() {
        let env_a = AgentEnvelope::new(
            AgentId::new("a"),
            Recipient::Direct(AgentId::new("b")),
            json!({}),
        );
        let env_b = AgentEnvelope::new(
            AgentId::new("a"),
            Recipient::Direct(AgentId::new("b")),
            json!({}),
        );
        assert_ne!(env_a.correlation_id, env_b.correlation_id);
    }

    #[test]
    fn reply_preserves_correlation_id() {
        let req = AgentEnvelope::new(
            AgentId::new("client"),
            Recipient::Direct(AgentId::new("server")),
            json!({"q": "ping"}),
        );
        let resp = AgentEnvelope::reply_to(&req, AgentId::new("server"), json!({"a": "pong"}));
        assert_eq!(req.correlation_id, resp.correlation_id);
        assert!(matches!(&resp.to, Recipient::Direct(id) if id.as_str() == "client"));
    }

    #[test]
    fn recipient_serde_roundtrip() {
        let r = Recipient::ByCapability {
            name: "network.firewall.address_list.add".into(),
            version_req: "^1".parse().unwrap(),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: Recipient = serde_json::from_str(&json).unwrap();
        match back {
            Recipient::ByCapability { name, version_req } => {
                assert_eq!(name, "network.firewall.address_list.add");
                assert!(version_req.matches(&semver::Version::new(1, 5, 0)));
            }
            _ => panic!("wrong variant"),
        }
    }
}
