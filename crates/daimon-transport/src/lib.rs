//! Shared transport library for daimon (D7). **INTERNAL — broker-only (D21).**
//!
//! Worker crates MUST NOT add `daimon-transport` to their `Cargo.toml`. Only
//! `daimon-broker` depends on this crate and enables `for-broker`. Without
//! that feature, the crate exports nothing.
//!
//! Phase 2 (this commit) ships:
//! - `Transport` trait + `Op`/`OpResult` types
//! - `StubTransport` — records calls, useful for broker testing
//! - SSH/REST/SNMP impl skeletons stubbed (real `russh`/`reqwest`/`csnmp`
//!   integration lands in the Phase 2 continuation session)

#[cfg(feature = "for-broker")]
pub mod op;
#[cfg(feature = "for-broker")]
pub mod transport;
#[cfg(feature = "for-broker")]
pub mod stub;

#[cfg(feature = "for-broker")]
pub use op::{HttpMethod, Op, OpResult, SnmpValue, TransportError};
#[cfg(feature = "for-broker")]
pub use transport::{Transport, TransportTarget};
#[cfg(feature = "for-broker")]
pub use stub::{StubTransport, StubTransportRecord};

#[cfg(all(test, not(feature = "for-broker")))]
mod op;
#[cfg(all(test, not(feature = "for-broker")))]
mod transport;
#[cfg(all(test, not(feature = "for-broker")))]
mod stub;
