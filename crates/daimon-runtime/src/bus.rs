use std::sync::Arc;

use async_trait::async_trait;
use daimon_core::{AgentEnvelope, CoreError};
use tokio::sync::broadcast;

/// Capacity of the broadcast channel used by `InProcBus`.
///
/// 1024 is comfortably larger than the active in-flight count we expect during
/// Phase 1 prototyping (~tens). Lagging receivers see broadcast lag errors,
/// which the supervisor logs but does not treat as fatal.
const IN_PROC_BUS_CAPACITY: usize = 1024;

/// The runtime-side bus trait. Extends `BusHandle` (from `daimon-core`) with
/// the subscribe + topology operations the supervisor needs.
///
/// In Phase 1 the only impl is `InProcBus`. Phase 8 will add `NatsBus` behind
/// the `nats` feature flag, with the same interface.
#[async_trait]
pub trait AgentBus: daimon_core::BusHandle + Send + Sync {
    /// Open a new envelope receiver. The supervisor gives each agent task its
    /// own receiver; the agent filters envelopes by `to` field.
    fn subscribe_raw(&self) -> broadcast::Receiver<AgentEnvelope>;
}

/// In-process bus backed by a single tokio broadcast channel.
///
/// Every envelope is published to every subscriber; each agent task filters by
/// the envelope's `to` field. This is wasteful at large N but trivially correct
/// for Phase 1 (~tens of agents) and uses the same primitive `daimon-app`
/// already uses for WebSocket broadcast.
#[derive(Clone)]
pub struct InProcBus {
    sender: broadcast::Sender<AgentEnvelope>,
}

impl InProcBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(IN_PROC_BUS_CAPACITY);
        Self { sender }
    }

    /// Wrap as a `BusHandle` trait object for handing to agents via context.
    pub fn handle(&self) -> Arc<dyn daimon_core::BusHandle> {
        Arc::new(self.clone())
    }
}

impl Default for InProcBus {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl daimon_core::BusHandle for InProcBus {
    async fn send(&self, env: AgentEnvelope) -> Result<(), CoreError> {
        // broadcast send only fails when there are zero receivers — that's a
        // valid "no one is listening" state, not an error. Drop the envelope
        // silently in that case to keep agent code simple.
        let _ = self.sender.send(env);
        Ok(())
    }
}

#[async_trait]
impl AgentBus for InProcBus {
    fn subscribe_raw(&self) -> broadcast::Receiver<AgentEnvelope> {
        self.sender.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daimon_core::{AgentEnvelope, AgentId, Recipient};
    use serde_json::json;

    #[tokio::test]
    async fn subscribe_then_send_delivers_envelope() {
        let bus = InProcBus::new();
        let mut rx = bus.subscribe_raw();
        let env = AgentEnvelope::new(
            AgentId::new("alpha"),
            Recipient::Direct(AgentId::new("beta")),
            json!({"ping": 1}),
        );
        daimon_core::BusHandle::send(&bus, env.clone())
            .await
            .unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received.correlation_id, env.correlation_id);
    }

    #[tokio::test]
    async fn send_with_no_subscribers_is_silent() {
        let bus = InProcBus::new();
        let env = AgentEnvelope::new(
            AgentId::new("alpha"),
            Recipient::Direct(AgentId::new("beta")),
            json!({}),
        );
        let res = daimon_core::BusHandle::send(&bus, env).await;
        assert!(res.is_ok());
    }
}
