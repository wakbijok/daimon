//! The send-and-await-correlation dispatch primitive (SDS §4.4).
//!
//! Extracted from `daimon-app`'s `Harness` so it can be shared by callers that
//! cannot depend on `daimon-app` — chiefly the orchestrator (D21: the
//! orchestrator dispatches over the bus but must NOT import the app). This is
//! the request/response primitive for callers (chat, orchestrator) that are not
//! themselves supervised agents.
//!
//! - Fails closed (`CapabilityNotFound`) if no registered agent satisfies
//!   `(name, version_req)` — BEFORE anything is sent (AC-P2-02).
//! - Subscribes to the bus before sending so a fast reply is never missed.
//! - The reply is the envelope carrying the same `correlation_id` addressed back
//!   to `from` (`Recipient::Direct(from)`); the caller's own outgoing request
//!   (addressed `ByCapability`) is skipped.

use std::time::Duration;

use daimon_core::{AgentEnvelope, AgentId, BusHandle, CoreError, Recipient};
use semver::VersionReq;
use thiserror::Error;
use tokio::sync::broadcast;

use crate::bus::{AgentBus, InProcBus};
use crate::registry::CapabilityRegistry;

/// Failure modes of a bus dispatch. Mirrors the old `daimon-app` `HarnessError`
/// (that type now delegates here and maps each variant across).
#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("no agent provides capability `{0}`")]
    CapabilityNotFound(String),
    #[error("dispatch timed out after {0:?}")]
    Timeout(Duration),
    #[error("bus closed before a reply arrived")]
    BusClosed,
    #[error("core: {0}")]
    Core(#[from] CoreError),
}

/// Routes a capability call over the bus and awaits its correlated reply.
///
/// Holds a bus handle + the capability registry; both are clone-cheap
/// (`InProcBus` is broadcast-backed, `CapabilityRegistry` is `Arc<RwLock<..>>`),
/// so `Dispatcher` itself is `Clone`. It owns no agent tasks — the supervisor
/// keeps those alive.
#[derive(Clone)]
pub struct Dispatcher {
    bus: InProcBus,
    registry: CapabilityRegistry,
}

impl Dispatcher {
    pub fn new(bus: InProcBus, registry: CapabilityRegistry) -> Self {
        Self { bus, registry }
    }

    /// The registry backing this dispatcher — exposed so callers (the saga
    /// compensation pass) can resolve a `Capability` (to read `compensating`)
    /// via the SAME registry the dispatch resolves against.
    pub fn registry(&self) -> &CapabilityRegistry {
        &self.registry
    }

    /// Route a capability call over the bus and await its correlated reply.
    ///
    /// See the module docs for the fail-closed / subscribe-before-send /
    /// correlation-match contract.
    pub async fn dispatch(
        &self,
        from: AgentId,
        name: &str,
        version_req: &VersionReq,
        body: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, DispatchError> {
        // 1. Resolve + fail closed (FR-HAR-09/10). No fallback to a
        //    version-incompatible agent, no fallback to broker.execute.
        self.registry
            .require_by_capability(name, version_req)
            .await
            .map_err(|_| DispatchError::CapabilityNotFound(format!("{name} @ {version_req}")))?;

        // 2. Subscribe BEFORE sending so we can't miss the reply.
        let mut rx = self.bus.subscribe_raw();
        let req = AgentEnvelope::new(
            from.clone(),
            Recipient::ByCapability {
                name: name.to_string(),
                version_req: version_req.clone(),
            },
            body,
        );
        let corr = req.correlation_id;
        self.bus.send(req).await?;

        // 3. Await the reply carrying our correlation id, bounded by timeout.
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(DispatchError::Timeout(timeout));
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Err(_) => return Err(DispatchError::Timeout(timeout)),
                Ok(Ok(env)) => {
                    if env.correlation_id == corr {
                        if let Recipient::Direct(ref id) = env.to {
                            if *id == from {
                                return Ok(env.body);
                            }
                        }
                        // else: our own outgoing ByCapability request — skip.
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => return Err(DispatchError::BusClosed),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Supervisor;
    use async_trait::async_trait;
    use daimon_core::{Agent, AgentContext, Capability};
    use semver::Version;
    use serde_json::json;
    use std::sync::Arc;

    /// A minimal agent that echoes any request back to its sender — enough to
    /// prove the dispatch primitive (correlation + fail-closed) without a broker.
    struct EchoAgent {
        id: AgentId,
        caps: Vec<Capability>,
    }

    #[async_trait]
    impl Agent for EchoAgent {
        fn id(&self) -> &AgentId {
            &self.id
        }
        fn capabilities(&self) -> &[Capability] {
            &self.caps
        }
        async fn handle(&self, env: AgentEnvelope, ctx: AgentContext) -> Result<(), CoreError> {
            let reply = AgentEnvelope::reply_to(&env, self.id.clone(), json!({ "echo": env.body }));
            ctx.bus.send(reply).await
        }
    }

    async fn dispatcher_with_echo() -> (Dispatcher, Arc<Supervisor>) {
        let bus = InProcBus::new();
        let registry = CapabilityRegistry::new();
        let supervisor = Arc::new(Supervisor::new(bus.clone(), registry.clone()));
        let echo = Arc::new(EchoAgent {
            id: AgentId::new("echo"),
            caps: vec![Capability::read_only("test.echo", Version::new(1, 0, 0))],
        });
        supervisor
            .spawn(echo as Arc<dyn Agent>)
            .await
            .expect("spawn echo");
        // Let the supervised runner subscribe to the broadcast before we send.
        tokio::time::sleep(Duration::from_millis(100)).await;
        (Dispatcher::new(bus, registry), supervisor)
    }

    #[tokio::test]
    async fn dispatch_round_trips_through_the_bus() {
        let (d, _sup) = dispatcher_with_echo().await;
        let req: VersionReq = "^1".parse().unwrap();
        let out = d
            .dispatch(
                AgentId::new("client"),
                "test.echo",
                &req,
                json!({ "ping": 1 }),
                Duration::from_secs(2),
            )
            .await
            .expect("dispatch");
        assert_eq!(out["echo"]["ping"], 1);
    }

    #[tokio::test]
    async fn dispatch_fails_closed_on_version_mismatch() {
        let (d, _sup) = dispatcher_with_echo().await;
        let req_v2: VersionReq = "^2".parse().unwrap();
        let err = d
            .dispatch(
                AgentId::new("client"),
                "test.echo",
                &req_v2,
                json!({}),
                Duration::from_secs(2),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DispatchError::CapabilityNotFound(_)));
    }

    #[tokio::test]
    async fn dispatch_times_out_when_no_reply() {
        // A capability that IS registered (so resolution passes) but whose
        // provider never replies (the registry entry has no live handler on the
        // bus) makes dispatch hit the timeout branch rather than fail-closed.
        let bus = InProcBus::new();
        let registry = CapabilityRegistry::new();
        registry
            .register(
                AgentId::new("silent"),
                vec![Capability::read_only("test.silent", Version::new(1, 0, 0))],
            )
            .await;
        let d = Dispatcher::new(bus, registry);
        let req: VersionReq = "^1".parse().unwrap();
        let err = d
            .dispatch(
                AgentId::new("client"),
                "test.silent",
                &req,
                json!({}),
                Duration::from_millis(150),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DispatchError::Timeout(_)));
    }
}
