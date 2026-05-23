//! Phase 2b #14 — server-fns backing `/admin/audit`.
//!
//! Read-only audit log viewer. Two server-fns: `list_audit_events` (paged) +
//! `count_audit_events` (for pagination math). Both gated by `require_admin()`
//! and forward to `broker.audit_query` / `broker.audit_count` (which do the
//! actual SQL query against the audit DB; D23).
//!
//! Wire DTOs mirror `daimon_audit` types so daimon-app keeps the D21 invariant
//! (no direct daimon-audit import — only via daimon_broker re-exports).
//!
//! Time filter uses `Option<i64>` epoch-seconds on the wire — minimal, no
//! timezone confusion, trivially constructed from `Date.getTime() / 1000` on
//! the client and `chrono::DateTime::from_timestamp(secs, 0)` on the server.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Wire mirror of `daimon_audit::ActionKind` (14 variants including
/// `InventoryResolve` added in #13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKindDto {
    BrokerExecute,
    VaultResolve,
    VaultReveal,
    VaultCreate,
    VaultUpdate,
    VaultRename,
    VaultDelete,
    InventoryUpsert,
    InventoryRemove,
    InventoryResolve,
    TransportDispatch,
    GuardApprove,
    GuardDeny,
    MemoryIngest,
    MemoryRetrieve,
    Other,
}

impl ActionKindDto {
    pub fn label(&self) -> &'static str {
        match self {
            Self::BrokerExecute => "broker.execute",
            Self::VaultResolve => "vault.resolve",
            Self::VaultReveal => "vault.reveal",
            Self::VaultCreate => "vault.create",
            Self::VaultUpdate => "vault.update",
            Self::VaultRename => "vault.rename",
            Self::VaultDelete => "vault.delete",
            Self::InventoryUpsert => "inventory.upsert",
            Self::InventoryRemove => "inventory.remove",
            Self::InventoryResolve => "inventory.resolve",
            Self::TransportDispatch => "transport.dispatch",
            Self::GuardApprove => "guard.approve",
            Self::GuardDeny => "guard.deny",
            Self::MemoryIngest => "memory.ingest",
            Self::MemoryRetrieve => "memory.retrieve",
            Self::Other => "other",
        }
    }

    pub fn all() -> [Self; 16] {
        [
            Self::BrokerExecute,
            Self::VaultResolve,
            Self::VaultReveal,
            Self::VaultCreate,
            Self::VaultUpdate,
            Self::VaultRename,
            Self::VaultDelete,
            Self::InventoryUpsert,
            Self::InventoryRemove,
            Self::InventoryResolve,
            Self::TransportDispatch,
            Self::GuardApprove,
            Self::GuardDeny,
            Self::MemoryIngest,
            Self::MemoryRetrieve,
            Self::Other,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditResultDto {
    Success,
    Error,
    Denied,
}

impl AuditResultDto {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
            Self::Denied => "denied",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditFilterDto {
    pub actor_id: Option<String>,
    pub action: Option<ActionKindDto>,
    pub target_ref: Option<String>,
    pub result: Option<AuditResultDto>,
    /// Epoch seconds (UTC). Inclusive lower bound.
    pub since_epoch_s: Option<i64>,
    /// Epoch seconds (UTC). Inclusive upper bound.
    pub until_epoch_s: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEventRow {
    pub id: uuid::Uuid,
    pub ts_rfc3339: String,
    pub actor_id: String,
    pub action: ActionKindDto,
    pub target_ref: Option<String>,
    pub credential_ref: Option<String>,
    pub op_summary: Option<String>,
    pub result: AuditResultDto,
    pub latency_ms: Option<u64>,
    pub metadata: BTreeMap<String, String>,
}

// -------- Server-side bridge: DTO <-> broker types ---------------------------

#[cfg(feature = "ssr")]
mod bridge {
    use super::*;
    use chrono::{DateTime, Utc};
    use daimon_broker::{ActionKind, AuditEvent, AuditFilter, AuditResult};

    impl From<ActionKind> for ActionKindDto {
        fn from(k: ActionKind) -> Self {
            match k {
                ActionKind::BrokerExecute => Self::BrokerExecute,
                ActionKind::VaultResolve => Self::VaultResolve,
                ActionKind::VaultReveal => Self::VaultReveal,
                ActionKind::VaultCreate => Self::VaultCreate,
                ActionKind::VaultUpdate => Self::VaultUpdate,
                ActionKind::VaultRename => Self::VaultRename,
                ActionKind::VaultDelete => Self::VaultDelete,
                ActionKind::InventoryUpsert => Self::InventoryUpsert,
                ActionKind::InventoryRemove => Self::InventoryRemove,
                ActionKind::InventoryResolve => Self::InventoryResolve,
                ActionKind::TransportDispatch => Self::TransportDispatch,
                ActionKind::GuardApprove => Self::GuardApprove,
                ActionKind::GuardDeny => Self::GuardDeny,
                ActionKind::MemoryIngest => Self::MemoryIngest,
                ActionKind::MemoryRetrieve => Self::MemoryRetrieve,
                ActionKind::Other => Self::Other,
            }
        }
    }

    impl From<ActionKindDto> for ActionKind {
        fn from(k: ActionKindDto) -> Self {
            match k {
                ActionKindDto::BrokerExecute => Self::BrokerExecute,
                ActionKindDto::VaultResolve => Self::VaultResolve,
                ActionKindDto::VaultReveal => Self::VaultReveal,
                ActionKindDto::VaultCreate => Self::VaultCreate,
                ActionKindDto::VaultUpdate => Self::VaultUpdate,
                ActionKindDto::VaultRename => Self::VaultRename,
                ActionKindDto::VaultDelete => Self::VaultDelete,
                ActionKindDto::InventoryUpsert => Self::InventoryUpsert,
                ActionKindDto::InventoryRemove => Self::InventoryRemove,
                ActionKindDto::InventoryResolve => Self::InventoryResolve,
                ActionKindDto::TransportDispatch => Self::TransportDispatch,
                ActionKindDto::GuardApprove => Self::GuardApprove,
                ActionKindDto::GuardDeny => Self::GuardDeny,
                ActionKindDto::MemoryIngest => Self::MemoryIngest,
                ActionKindDto::MemoryRetrieve => Self::MemoryRetrieve,
                ActionKindDto::Other => Self::Other,
            }
        }
    }

    impl From<AuditResult> for AuditResultDto {
        fn from(r: AuditResult) -> Self {
            match r {
                AuditResult::Success => Self::Success,
                AuditResult::Error => Self::Error,
                AuditResult::Denied => Self::Denied,
            }
        }
    }

    impl From<AuditResultDto> for AuditResult {
        fn from(r: AuditResultDto) -> Self {
            match r {
                AuditResultDto::Success => Self::Success,
                AuditResultDto::Error => Self::Error,
                AuditResultDto::Denied => Self::Denied,
            }
        }
    }

    impl From<AuditEvent> for AuditEventRow {
        fn from(e: AuditEvent) -> Self {
            Self {
                id: e.id,
                ts_rfc3339: e.ts.to_rfc3339(),
                actor_id: e.actor_id,
                action: e.action.into(),
                target_ref: e.target_ref,
                credential_ref: e.credential_ref,
                op_summary: e.op_summary,
                result: e.result.into(),
                latency_ms: e.latency_ms,
                metadata: e.metadata,
            }
        }
    }

    /// Build a broker-side `AuditFilter` from the wire DTO. Epoch-seconds
    /// convert to `DateTime<Utc>` via `from_timestamp`.
    pub fn dto_to_filter(d: &AuditFilterDto) -> AuditFilter {
        let to_dt = |s: Option<i64>| -> Option<DateTime<Utc>> {
            s.and_then(|secs| DateTime::from_timestamp(secs, 0))
        };
        AuditFilter {
            actor_id: d.actor_id.clone().filter(|s| !s.is_empty()),
            action: d.action.map(Into::into),
            target_ref: d.target_ref.clone().filter(|s| !s.is_empty()),
            result: d.result.map(Into::into),
            since: to_dt(d.since_epoch_s),
            until: to_dt(d.until_epoch_s),
        }
    }
}

// -------- Server-fns ---------------------------------------------------------

/// Server-fn args flattened — passing the filter struct directly was hitting
/// a Leptos 0.8 serialization quirk where the WASM client body arrived
/// without the top-level `filter` field. Flat individual params avoid the
/// nested-struct path entirely.
#[server]
pub async fn list_audit_events(
    actor_id: Option<String>,
    action: Option<ActionKindDto>,
    target_ref: Option<String>,
    result: Option<AuditResultDto>,
    since_epoch_s: Option<i64>,
    until_epoch_s: Option<i64>,
    limit: u32,
    offset: u32,
) -> Result<Vec<AuditEventRow>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let filter = AuditFilterDto {
        actor_id,
        action,
        target_ref,
        result,
        since_epoch_s,
        until_epoch_s,
    };
    let broker_filter = bridge::dto_to_filter(&filter);
    let events = state
        .broker
        .audit_query(&claims.sub, &broker_filter, limit, offset)
        .await
        .map_err(|e| ServerFnError::new(format!("audit_query: {e}")))?;
    Ok(events.into_iter().map(AuditEventRow::from).collect())
}

#[server]
pub async fn count_audit_events(
    actor_id: Option<String>,
    action: Option<ActionKindDto>,
    target_ref: Option<String>,
    result: Option<AuditResultDto>,
    since_epoch_s: Option<i64>,
    until_epoch_s: Option<i64>,
) -> Result<u64, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let filter = AuditFilterDto {
        actor_id,
        action,
        target_ref,
        result,
        since_epoch_s,
        until_epoch_s,
    };
    let broker_filter = bridge::dto_to_filter(&filter);
    state
        .broker
        .audit_count(&claims.sub, &broker_filter)
        .await
        .map_err(|e| ServerFnError::new(format!("audit_count: {e}")))
}
