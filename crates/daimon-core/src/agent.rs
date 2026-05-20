use async_trait::async_trait;
use std::sync::Arc;

use crate::capability::Capability;
use crate::envelope::AgentEnvelope;
use crate::error::CoreError;
use crate::id::AgentId;

/// The Agent contract.
///
/// Agents are spawned and supervised by `daimon-runtime`. They handle incoming
/// envelopes, optionally send replies via `ctx.send`, and may delegate to
/// other agents on the bus. `AgentContext` is injected per call rather than
/// per instance so agents are stateless from the runtime's perspective and
/// can be restarted in place.
#[async_trait]
pub trait Agent: Send + Sync {
    fn id(&self) -> &AgentId;
    fn capabilities(&self) -> &[Capability];
    async fn handle(&self, env: AgentEnvelope, ctx: AgentContext) -> Result<(), CoreError>;
}

/// Per-call context handed to an agent's `handle` method.
///
/// Carries the bus handle (for replies / delegation) and the agent's own
/// identity for convenience in logs. Phase 3+ extends this with memory and
/// vault handles; Phase 5+ adds audit logger.
#[derive(Clone)]
pub struct AgentContext {
    pub agent_id: AgentId,
    pub bus: Arc<dyn BusHandle>,
}

impl AgentContext {
    pub fn new(agent_id: AgentId, bus: Arc<dyn BusHandle>) -> Self {
        Self { agent_id, bus }
    }
}

/// Minimal bus handle exposed to agents through `AgentContext`.
///
/// The concrete `AgentBus` lives in `daimon-runtime`; this trait is the
/// abstract slice agents need so `daimon-core` stays free of runtime deps.
#[async_trait]
pub trait BusHandle: Send + Sync {
    async fn send(&self, env: AgentEnvelope) -> Result<(), CoreError>;
}
