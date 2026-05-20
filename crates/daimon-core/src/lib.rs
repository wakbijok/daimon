//! Core types and traits for the daimon multi-agent runtime.
//!
//! This crate is the dependency root of the agent system. No I/O, no async
//! runtime tied in — every other daimon crate imports from here.
//!
//! See `docs/specs/2026-05-20-multi-agent-architecture-design.md` for the
//! architectural context (D17 versioning, D18 saga rollback).

pub mod agent;
pub mod capability;
pub mod envelope;
pub mod error;
pub mod id;

pub use agent::{Agent, AgentContext, BusHandle};
pub use capability::{Capability, CompensatingCapability};
pub use envelope::{AgentEnvelope, AuditMetadata, Recipient};
pub use error::CoreError;
pub use id::AgentId;
