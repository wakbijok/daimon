//! Managed asset registry (D20). **INTERNAL — broker-only (D21).**
//!
//! Worker crates MUST NOT add `daimon-inventory` to their `Cargo.toml`. Only
//! `daimon-broker` depends on this crate and enables `for-broker`. Without
//! that feature, the crate exports nothing.
//!
//! Maps `target://<name>` references to `ManagedTarget` records — host, port,
//! transport type, credential ref, kind, labels, capabilities. The credential
//! ref (`vault://...`) never leaves this crate; broker reads it internally
//! before dispatching transport. Workers see only `TargetMetadata` (no
//! credential field).
//!
//! Two `Inventory` impls:
//! - `InMemoryRegistry` — useful for tests + dev workflows
//! - `PostgresRegistry` — production default (Phase 2c). Replaces the prior
//!   `SqliteRegistry` (D3b rip-and-replace).
//!
//! See `daimon-docs/specs/2026-05-20-multi-agent-architecture-design.md` D19/D20/D21.

#[cfg(feature = "for-broker")]
pub mod postgres_registry;
#[cfg(feature = "for-broker")]
pub mod refspec;
#[cfg(feature = "for-broker")]
pub mod registry;
#[cfg(feature = "for-broker")]
pub mod target;

#[cfg(feature = "for-broker")]
pub use postgres_registry::PostgresRegistry;
#[cfg(feature = "for-broker")]
pub use refspec::{RefParseError, TargetRef};
#[cfg(feature = "for-broker")]
pub use registry::{InMemoryRegistry, Inventory, InventoryError};
#[cfg(feature = "for-broker")]
pub use target::{ManagedTarget, TargetKind, TargetMetadata, TransportKind};

#[cfg(all(test, not(feature = "for-broker")))]
mod refspec;
#[cfg(all(test, not(feature = "for-broker")))]
mod registry;
#[cfg(all(test, not(feature = "for-broker")))]
mod target;
