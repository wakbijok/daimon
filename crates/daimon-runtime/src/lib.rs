//! Agent lifecycle and message routing for the daimon multi-agent runtime.
//!
//! Hosts the `AgentBus` trait and concrete implementations. Phase 1 ships
//! `InProcBus` backed by tokio broadcast channels for in-process agent
//! communication. Phase 8 adds `NatsBus` behind the `nats` feature flag for
//! distribution-ready deployments.
//!
//! Also home of the `Supervisor` (spawn / restart / healthcheck) and
//! `CapabilityRegistry` (agent capability discovery).
//!
//! Phase 0 ships an empty skeleton. See `docs/specs/2026-05-20-multi-agent-architecture-design.md`.
