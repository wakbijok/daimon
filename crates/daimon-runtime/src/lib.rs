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
pub mod error;
pub mod registry;
pub mod supervisor;

pub use bus::{AgentBus, InProcBus};
pub use error::RuntimeError;
pub use registry::{CapabilityRegistry, RegistryEntry};
pub use supervisor::{Supervisor, SupervisorConfig};
