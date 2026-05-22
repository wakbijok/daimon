use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A canonical action recorded in the audit log. Free-form strings are
/// rejected at the API boundary to keep audit queries semantically reliable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Broker received an `execute` call (entry point).
    BrokerExecute,
    /// Vault resolve completed (broker's internal step).
    VaultResolve,
    /// Vault reveal — admin viewed a credential's full payload.
    VaultReveal,
    /// Vault create — admin created a credential.
    VaultCreate,
    /// Vault update — admin updated a credential.
    VaultUpdate,
    /// Vault rename — admin renamed a credential.
    VaultRename,
    /// Vault delete — admin deleted a credential.
    VaultDelete,
    /// Inventory upsert — admin added/updated a managed target.
    InventoryUpsert,
    /// Inventory remove — admin removed a managed target.
    InventoryRemove,
    /// Inventory resolve — admin read a managed target's full record
    /// (including credential_ref). Audited because credential_ref is
    /// sensitive — it tells the operator which vault entry binds where.
    InventoryResolve,
    /// Transport dispatch — broker dispatched an Op to a transport impl.
    TransportDispatch,
    /// Guard decision (Phase 5+).
    GuardApprove,
    GuardDeny,
    /// Memory tier — document ingested into long-term collection (Phase 3).
    MemoryIngest,
    /// Memory tier — long-term retrieval performed (Phase 3).
    MemoryRetrieve,
    /// Generic — for cases not covered above.
    Other,
}

impl ActionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ActionKind::BrokerExecute => "broker.execute",
            ActionKind::VaultResolve => "vault.resolve",
            ActionKind::VaultReveal => "vault.reveal",
            ActionKind::VaultCreate => "vault.create",
            ActionKind::VaultUpdate => "vault.update",
            ActionKind::VaultRename => "vault.rename",
            ActionKind::VaultDelete => "vault.delete",
            ActionKind::InventoryUpsert => "inventory.upsert",
            ActionKind::InventoryRemove => "inventory.remove",
            ActionKind::InventoryResolve => "inventory.resolve",
            ActionKind::TransportDispatch => "transport.dispatch",
            ActionKind::GuardApprove => "guard.approve",
            ActionKind::GuardDeny => "guard.deny",
            ActionKind::MemoryIngest => "memory.ingest",
            ActionKind::MemoryRetrieve => "memory.retrieve",
            ActionKind::Other => "other",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "broker.execute" => ActionKind::BrokerExecute,
            "vault.resolve" => ActionKind::VaultResolve,
            "vault.reveal" => ActionKind::VaultReveal,
            "vault.create" => ActionKind::VaultCreate,
            "vault.update" => ActionKind::VaultUpdate,
            "vault.rename" => ActionKind::VaultRename,
            "vault.delete" => ActionKind::VaultDelete,
            "inventory.upsert" => ActionKind::InventoryUpsert,
            "inventory.remove" => ActionKind::InventoryRemove,
            "inventory.resolve" => ActionKind::InventoryResolve,
            "transport.dispatch" => ActionKind::TransportDispatch,
            "guard.approve" => ActionKind::GuardApprove,
            "guard.deny" => ActionKind::GuardDeny,
            "memory.ingest" => ActionKind::MemoryIngest,
            "memory.retrieve" => ActionKind::MemoryRetrieve,
            _ => ActionKind::Other,
        }
    }
}

/// Result tag for an audited action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditResult {
    Success,
    Error,
    Denied,
}

impl AuditResult {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuditResult::Success => "success",
            AuditResult::Error => "error",
            AuditResult::Denied => "denied",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "success" => AuditResult::Success,
            "error" => AuditResult::Error,
            "denied" => AuditResult::Denied,
            _ => AuditResult::Error,
        }
    }
}

/// An event ready to be appended. The sink assigns `id` + `ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewAuditEvent {
    pub actor_id: String,
    pub action: ActionKind,
    pub target_ref: Option<String>,
    pub credential_ref: Option<String>,
    pub op_summary: Option<String>,
    pub result: AuditResult,
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl NewAuditEvent {
    pub fn new(actor_id: impl Into<String>, action: ActionKind, result: AuditResult) -> Self {
        Self {
            actor_id: actor_id.into(),
            action,
            target_ref: None,
            credential_ref: None,
            op_summary: None,
            result,
            latency_ms: None,
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target_ref = Some(target.into());
        self
    }

    pub fn with_credential(mut self, cred: impl Into<String>) -> Self {
        self.credential_ref = Some(cred.into());
        self
    }

    pub fn with_op_summary(mut self, s: impl Into<String>) -> Self {
        self.op_summary = Some(s.into());
        self
    }

    pub fn with_latency_ms(mut self, ms: u64) -> Self {
        self.latency_ms = Some(ms);
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// A persisted audit event. `id` and `ts` are filled in by the sink.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: i64,
    pub ts: DateTime<Utc>,
    pub actor_id: String,
    pub action: ActionKind,
    pub target_ref: Option<String>,
    pub credential_ref: Option<String>,
    pub op_summary: Option<String>,
    pub result: AuditResult,
    pub latency_ms: Option<u64>,
    pub metadata: BTreeMap<String, String>,
}

/// Filter for `AuditSink::query`. All fields are optional; setting multiple
/// fields ANDs them together.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditFilter {
    pub actor_id: Option<String>,
    pub action: Option<ActionKind>,
    pub target_ref: Option<String>,
    pub result: Option<AuditResult>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
}
