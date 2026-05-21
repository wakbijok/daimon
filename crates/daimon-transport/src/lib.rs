//! Shared transport library for daimon (D7). **INTERNAL — broker-only (D21).**
//!
//! Worker crates MUST NOT add `daimon-transport` to their `Cargo.toml`. Only
//! `daimon-broker` depends on this crate and enables `for-broker`. Without
//! that feature, the crate exports nothing.
//!
//! Phase 2a (commit `1acb0cc`) shipped:
//! - `Transport` trait + `Op`/`OpResult` types
//! - `StubTransport` — records calls, useful for broker testing
//!
//! Phase 2b ships real impls:
//! - `SshTransport` via `russh` — host-key verified by KnownHosts by default
//! - (next) `RestTransport` via `reqwest`
//! - (next) `SnmpTransport` via `csnmp`

#[cfg(feature = "for-broker")]
pub mod op;
#[cfg(feature = "for-broker")]
pub mod ssh;
#[cfg(feature = "for-broker")]
pub mod stub;
#[cfg(feature = "for-broker")]
pub mod transport;

#[cfg(feature = "for-broker")]
pub use op::{HttpMethod, Op, OpResult, SnmpValue, TransportError};
#[cfg(feature = "for-broker")]
pub use ssh::{HostKeyPolicy, SshTransport};
#[cfg(feature = "for-broker")]
pub use stub::{StubTransport, StubTransportRecord};
#[cfg(feature = "for-broker")]
pub use transport::{Transport, TransportTarget};

#[cfg(all(test, not(feature = "for-broker")))]
mod op;
#[cfg(all(test, not(feature = "for-broker")))]
mod ssh;
#[cfg(all(test, not(feature = "for-broker")))]
mod stub;
#[cfg(all(test, not(feature = "for-broker")))]
mod transport;
