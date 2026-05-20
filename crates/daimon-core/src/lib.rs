//! Core types and traits for the daimon multi-agent runtime.
//!
//! This crate is the dependency root of the agent system. It defines the
//! `Agent` trait, message envelope, capability descriptor, and shared error
//! types that every other daimon crate imports.
//!
//! Phase 0 ships an empty skeleton. Phase 1 populates this crate with the
//! `Agent` trait, `AgentId`, `AgentMessage`, `AgentEnvelope`, `Capability`,
//! and `AgentContext` types. See `docs/specs/2026-05-20-multi-agent-architecture-design.md`.
