//! Phase 2b #13 — server-fns backing `/admin/targets`.
//!
//! Four thin admin-gate-then-forward wrappers over `Broker::inventory_*`.
//! State-changing calls audit on the broker side (D23). D21 holds: no direct
//! daimon-inventory import — wire DTOs mirror broker types and convert only
//! under `#[cfg(feature = "ssr")]`.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Wire kind discriminator. Serde rep matches `daimon_inventory::TargetKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKindDto {
    Platform,
    Network,
    Host,
    App,
}

impl TargetKindDto {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Platform => "Platform",
            Self::Network => "Network",
            Self::Host => "Host",
            Self::App => "App",
        }
    }

    /// UI-9 — which console dashboard a target of this kind appears on. This is
    /// the SAME mapping `pages/class_dashboard.rs` filters by (Infrastructure =
    /// [Host, App], Network = [Network], Kubernetes = [Platform]) — surfaced at
    /// registration time so the operator knows where the endpoint will land.
    pub fn console_home(&self) -> &'static str {
        match self {
            Self::Platform => "Kubernetes",
            Self::Network => "Network",
            Self::Host | Self::App => "Infrastructure",
        }
    }

    /// One-line registration hint per kind — what this class means and how the
    /// console reaches it.
    pub fn console_hint(&self) -> &'static str {
        match self {
            Self::Platform => {
                "Kubernetes / orchestrator endpoints (kubectl via kubeconfig, cluster API)."
            }
            Self::Network => "Network + firewall devices (RouterOS, switches — SSH/SNMP/REST).",
            Self::Host => "Baremetal / VM / mini-PC hosts (SSH).",
            Self::App => "Application-level endpoints running on hosts (REST/gRPC).",
        }
    }

    pub fn all() -> [Self; 4] {
        [Self::Platform, Self::Network, Self::Host, Self::App]
    }
}

/// Wire transport discriminator. Matches `daimon_inventory::TransportKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKindDto {
    Ssh,
    Rest,
    Snmp,
    Grpc,
}

impl TransportKindDto {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Ssh => "SSH",
            Self::Rest => "REST",
            Self::Snmp => "SNMP",
            Self::Grpc => "gRPC",
        }
    }

    pub fn all() -> [Self; 4] {
        [Self::Ssh, Self::Rest, Self::Snmp, Self::Grpc]
    }

    /// Conventional default port per transport. Used by the form when the
    /// admin hasn't touched the port field.
    pub fn default_port(&self) -> u16 {
        match self {
            Self::Ssh => 22,
            Self::Rest => 443,
            Self::Snmp => 161,
            Self::Grpc => 50051,
        }
    }
}

/// List row — metadata fields only. `credential_ref` is **NOT** carried here
/// (the broker's `inventory_list` returns `TargetMetadata` which strips it).
/// The admin sees the credential binding only via Edit (which fetches the
/// full record via `get_target`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetRow {
    pub ref_name: String,
    pub kind: TargetKindDto,
    pub transport: TransportKindDto,
    pub host: String,
    pub port: u16,
    pub label_count: usize,
    pub capability_count: usize,
}

/// Full target shape used by Add + Edit forms and the get_target response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetDto {
    pub ref_name: String,
    pub kind: TargetKindDto,
    pub transport: TransportKindDto,
    pub host: String,
    pub port: u16,
    pub credential_ref: String,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// Preserved on edit, hidden from the UI in v1 (per #13 plan).
    #[serde(default)]
    pub capabilities: Vec<String>,
}

// -------- Server-side bridge: DTO <-> broker types ---------------------------

#[cfg(feature = "ssr")]
mod bridge {
    use super::*;
    use daimon_broker::{ManagedTarget, TargetKind, TargetMetadata, TargetRef, TransportKind};

    impl From<TargetKind> for TargetKindDto {
        fn from(k: TargetKind) -> Self {
            match k {
                TargetKind::Platform => Self::Platform,
                TargetKind::Network => Self::Network,
                TargetKind::Host => Self::Host,
                TargetKind::App => Self::App,
            }
        }
    }

    impl From<TargetKindDto> for TargetKind {
        fn from(k: TargetKindDto) -> Self {
            match k {
                TargetKindDto::Platform => Self::Platform,
                TargetKindDto::Network => Self::Network,
                TargetKindDto::Host => Self::Host,
                TargetKindDto::App => Self::App,
            }
        }
    }

    impl From<TransportKind> for TransportKindDto {
        fn from(t: TransportKind) -> Self {
            match t {
                TransportKind::Ssh => Self::Ssh,
                TransportKind::Rest => Self::Rest,
                TransportKind::Snmp => Self::Snmp,
                TransportKind::Grpc => Self::Grpc,
            }
        }
    }

    impl From<TransportKindDto> for TransportKind {
        fn from(t: TransportKindDto) -> Self {
            match t {
                TransportKindDto::Ssh => Self::Ssh,
                TransportKindDto::Rest => Self::Rest,
                TransportKindDto::Snmp => Self::Snmp,
                TransportKindDto::Grpc => Self::Grpc,
            }
        }
    }

    impl From<TargetMetadata> for TargetRow {
        fn from(m: TargetMetadata) -> Self {
            Self {
                ref_name: m.r#ref.name().to_string(),
                kind: m.kind.into(),
                transport: m.transport.into(),
                host: m.host,
                port: m.port,
                label_count: m.labels.len(),
                capability_count: m.capabilities.len(),
            }
        }
    }

    impl From<ManagedTarget> for TargetDto {
        fn from(t: ManagedTarget) -> Self {
            Self {
                ref_name: t.r#ref.name().to_string(),
                kind: t.kind.into(),
                transport: t.transport.into(),
                host: t.host,
                port: t.port,
                credential_ref: t.credential_ref,
                labels: t.labels,
                capabilities: t.capabilities,
            }
        }
    }

    /// Build a `ManagedTarget` from the wire DTO. Parses the ref name into
    /// `target://name`; surfaces ref-shape errors as ServerFnError at the
    /// server-fn boundary.
    pub fn dto_to_managed(d: TargetDto) -> Result<ManagedTarget, String> {
        let ref_str = format!("target://{}", d.ref_name);
        let r = TargetRef::parse(&ref_str).map_err(|e| format!("invalid ref: {e}"))?;
        Ok(ManagedTarget {
            r#ref: r,
            kind: d.kind.into(),
            transport: d.transport.into(),
            host: d.host,
            port: d.port,
            credential_ref: d.credential_ref,
            labels: d.labels,
            capabilities: d.capabilities,
        })
    }

    pub fn parse_ref(name: &str) -> Result<TargetRef, String> {
        TargetRef::parse(&format!("target://{name}")).map_err(|e| format!("invalid ref: {e}"))
    }
}

// -------- Server-fns ---------------------------------------------------------

#[server]
pub async fn list_targets() -> Result<Vec<TargetRow>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let rows = state.broker.inventory_list(&claims.sub, None).await;
    Ok(rows.into_iter().map(TargetRow::from).collect())
}

#[server]
pub async fn get_target(ref_name: String) -> Result<TargetDto, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let r = bridge::parse_ref(&ref_name).map_err(ServerFnError::new)?;
    let mt = state
        .broker
        .inventory_get_managed(&claims.sub, &r)
        .await
        .map_err(|e| ServerFnError::new(format!("inventory_get_managed: {e}")))?;
    Ok(mt.into())
}

#[server]
pub async fn upsert_target(target: TargetDto) -> Result<(), ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let mt = bridge::dto_to_managed(target).map_err(ServerFnError::new)?;
    state
        .broker
        .inventory_upsert(&claims.sub, mt)
        .await
        .map_err(|e| ServerFnError::new(format!("inventory_upsert: {e}")))
}

#[server]
pub async fn delete_target(ref_name: String) -> Result<(), ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let r = bridge::parse_ref(&ref_name).map_err(ServerFnError::new)?;
    state
        .broker
        .inventory_remove(&claims.sub, &r)
        .await
        .map_err(|e| ServerFnError::new(format!("inventory_remove: {e}")))
}
