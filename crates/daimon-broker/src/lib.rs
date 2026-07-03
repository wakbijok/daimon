//! Public action surface for daimon agents (D19) + admin proxy for daimon-app
//! (D22/D23/D24).
//!
//! Workers depend on this crate (and `daimon-core`) — never on `daimon-vault`,
//! `daimon-inventory`, `daimon-transport`, or `daimon-audit`. They construct
//! `ExecRequest`s and the broker handles credential resolution + transport
//! dispatch internally. Credentials never reach worker memory.
//!
//! `daimon-app` (the I/O adapter) also depends only on this crate for its
//! admin surface — `vault_*`, `inventory_*`, `audit_*` methods on `Broker`.
//! Per D21, daimon-app does NOT depend on vault/inventory/transport/audit
//! directly.
//!
//! Re-exports the public types from `daimon-inventory`, `daimon-transport`,
//! `daimon-vault`, and `daimon-audit` so workers + daimon-app depend only on
//! daimon-broker.
//!
//! See `docs/specs/2026-05-20-multi-agent-architecture-design.md`
//! D19/D20/D21/D22/D23/D24.

pub mod admin;
pub mod broker;
pub mod production;
pub mod request;

pub use broker::{Broker, BrokerError};
pub use request::ExecRequest;

// Re-export public types from internal crates so workers + daimon-app
// depend only on daimon-broker.
pub use daimon_audit::{ActionKind, AuditEvent, AuditFilter, AuditResult, NewAuditEvent};
pub use daimon_inventory::{
    Inventory, ManagedTarget, RefParseError as TargetRefParseError, TargetKind, TargetMetadata,
    TargetRef, TransportKind,
};
pub use daimon_transport::{AuthScheme, HttpMethod, Op, OpResult, SnmpValue};
pub use daimon_vault::{Credential, CredentialKind, CredentialMetadata, CredentialRef};
