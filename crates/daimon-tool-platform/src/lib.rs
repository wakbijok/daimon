//! Platform worker tier (Phase 7 D1).
//!
//! Capability-based core: every Platform impl knows how to list/get
//! workloads. Side-traits cover platform-specific writes (snapshot, clone,
//! live-migrate). The trait split lets the orchestrator pick capability
//! sets per-platform without leaking PVE/K8s/OpenStack specifics into the
//! agent code.
//!
//! Phase 7 ships the PVE driver. Phase 7.1+ adds K8s + OpenStack drivers.
//! Write capabilities (start/stop/snapshot/clone) get Guard-gated when
//! they're wired through the broker — Phase 7 ships read-only paths.

pub mod error;
pub mod platform;
pub mod poller;
pub mod pve;

pub use error::{Error, Result};
pub use platform::{Cloneable, Platform, Snapshotable, Snapshot, Workload, WorkloadKind};
pub use poller::{PlatformPoller, PollerConfig};
pub use pve::PveDriver;
