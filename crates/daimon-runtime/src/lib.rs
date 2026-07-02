//! Agent lifecycle and message routing for the daimon multi-agent runtime.
//!
//! Phase 1 ships:
//! - `AgentBus` trait + `InProcBus` impl (tokio broadcast)
//! - `CapabilityRegistry` — agent capability discovery, version-aware lookup (D17)
//! - `Supervisor` — spawn / restart / healthcheck
//!
//! Phase 8 adds `NatsBus` behind the `nats` feature flag.
//!
//! See `docs/specs/2026-05-20-multi-agent-architecture-design.md`.

pub mod bus;
pub mod dispatcher;
pub mod error;
#[cfg(feature = "nats")]
pub mod nats_bus;
pub mod registry;
pub mod supervisor;

pub use bus::{AgentBus, InProcBus};
pub use dispatcher::{DispatchError, Dispatcher};
pub use error::RuntimeError;
#[cfg(feature = "nats")]
pub use nats_bus::{NatsBus, NatsBusError};
pub use registry::{CapabilityRegistry, RegistryEntry};
pub use supervisor::{Supervisor, SupervisorConfig};
