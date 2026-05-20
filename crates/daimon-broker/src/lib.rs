//! Public action surface for daimon agents (D19).
//!
//! Workers depend on this crate (and `daimon-core`) — never on `daimon-vault`,
//! `daimon-inventory`, or `daimon-transport`. They construct `ExecRequest`s
//! and the broker handles credential resolution + transport dispatch
//! internally. Credentials never reach worker memory.
//!
//! Re-exports the agent-visible types from `daimon-inventory` and
//! `daimon-transport`: `TargetRef`, `TargetMetadata`, `Op`, `OpResult`. The
//! broker is the agent-facing wrapper.
//!
//! See `docs/specs/2026-05-20-multi-agent-architecture-design.md` D19/D20/D21.

pub mod broker;
pub mod request;

pub use broker::{Broker, BrokerError};
pub use request::ExecRequest;

// Re-export the public types from internal crates so workers depend only on
// daimon-broker. (Workers' Cargo.toml never lists daimon-vault, daimon-inventory,
// or daimon-transport — D21.)
pub use daimon_inventory::{
    Inventory, RefParseError as TargetRefParseError, TargetKind, TargetMetadata, TargetRef,
    TransportKind,
};
pub use daimon_transport::{HttpMethod, Op, OpResult, SnmpValue};
