//! The multi-agent harness — the live wiring of `daimon-runtime` (SDS §4).
//!
//! Before P2 the runtime (bus/registry/supervisor) had ZERO production
//! consumers; the app dispatched straight to `Broker::execute`. This module
//! constructs the runtime at boot, spawns the driver agents under the
//! supervisor, and exposes `dispatch` — the send-and-await-correlation
//! primitive that routes a capability call over the bus (`Recipient::ByCapability`),
//! resolving the provider via the registry and FAILING CLOSED when no agent
//! satisfies the version requirement (AC-P2-02).
//!
//! ssr-only: the runtime types are server-side. Held in `AppState`.
#![cfg(feature = "ssr")]

use std::sync::Arc;
use std::time::Duration;

use daimon_core::{AgentEnvelope, AgentId, BusHandle, Capability, CoreError, Recipient};
use daimon_runtime::{AgentBus, CapabilityRegistry, InProcBus, Supervisor};
use semver::VersionReq;
use thiserror::Error;
use tokio::sync::broadcast;

#[derive(Debug, Error)]
pub enum HarnessError {
    #[error("no agent provides capability `{0}`")]
    CapabilityNotFound(String),
    #[error("dispatch timed out after {0:?}")]
    Timeout(Duration),
    #[error("bus closed before a reply arrived")]
    BusClosed,
    #[error("core: {0}")]
    Core(#[from] CoreError),
}

/// The live harness held in `AppState`. Clone-cheap (all handles are `Arc`/
/// broadcast-backed). Owns the supervisor so the spawned agent tasks stay
/// alive for the process lifetime.
#[derive(Clone)]
pub struct Harness {
    bus: InProcBus,
    registry: CapabilityRegistry,
    _supervisor: Arc<Supervisor>,
}

impl Harness {
    pub fn new(bus: InProcBus, registry: CapabilityRegistry, supervisor: Arc<Supervisor>) -> Self {
        Self {
            bus,
            registry,
            _supervisor: supervisor,
        }
    }

    /// Route a capability call over the bus and await its correlated reply.
    ///
    /// - Fails closed (`CapabilityNotFound`) if no registered agent satisfies
    ///   `name` at `version_req` — BEFORE anything is sent (AC-P2-02).
    /// - Subscribes to the bus before sending so a fast reply is never missed.
    /// - The reply is the envelope carrying the same `correlation_id` addressed
    ///   back to `from` (`Recipient::Direct(from)`); our own outgoing request
    ///   (addressed `ByCapability`) is skipped.
    pub async fn dispatch(
        &self,
        from: AgentId,
        name: &str,
        version_req: &VersionReq,
        body: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, HarnessError> {
        self.registry
            .require_by_capability(name, version_req)
            .await
            .map_err(|_| HarnessError::CapabilityNotFound(format!("{name} @ {version_req}")))?;

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

        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(HarnessError::Timeout(timeout));
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Err(_) => return Err(HarnessError::Timeout(timeout)),
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
                Ok(Err(broadcast::error::RecvError::Closed)) => return Err(HarnessError::BusClosed),
            }
        }
    }

    /// Every capability currently registered (flattened across agents) — the
    /// source for the chat tool catalog + orchestrator planner catalog
    /// (projected at the call sites in P2 commits 5/6).
    pub async fn capabilities(&self) -> Vec<Capability> {
        self.registry
            .all()
            .await
            .into_iter()
            .flat_map(|e| e.capabilities)
            .collect()
    }

    /// The version requirement to dispatch a capability by name: caret of the
    /// highest registered version (`^x.y.z`), or `None` if unregistered.
    pub async fn version_req_for(&self, name: &str) -> Option<VersionReq> {
        let v = self
            .capabilities()
            .await
            .into_iter()
            .filter(|c| c.name == name)
            .map(|c| c.version)
            .max()?;
        format!("^{v}").parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use daimon_core::{Agent, AgentContext};
    use semver::Version;
    use serde_json::json;

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

    async fn harness_with_echo() -> Harness {
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
        Harness::new(bus, registry, supervisor)
    }

    #[tokio::test]
    async fn dispatch_round_trips_through_the_bus() {
        let h = harness_with_echo().await;
        let req: VersionReq = "^1".parse().unwrap();
        let out = h
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
        let h = harness_with_echo().await;
        let req_v2: VersionReq = "^2".parse().unwrap();
        let err = h
            .dispatch(
                AgentId::new("client"),
                "test.echo",
                &req_v2,
                json!({}),
                Duration::from_secs(2),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, HarnessError::CapabilityNotFound(_)));
    }

    #[tokio::test]
    async fn version_req_for_returns_caret_of_registered() {
        let h = harness_with_echo().await;
        let req = h.version_req_for("test.echo").await.expect("some");
        assert!(req.matches(&Version::new(1, 4, 0)));
        assert!(!req.matches(&Version::new(2, 0, 0)));
    }
}
