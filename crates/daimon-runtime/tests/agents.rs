//! Phase 1 acceptance tests for the multi-agent runtime.
//!
//! Two trivial agents (Echo and Ping) exercise:
//! - capability registration + version-aware discovery
//! - direct and by-capability addressing
//! - roundtrip envelope delivery
//! - supervisor restart after a panic (agent resumes serving)

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use daimon_core::{
    Agent, AgentContext, AgentEnvelope, AgentId, BusHandle, Capability, CoreError, Recipient,
};
use daimon_runtime::{CapabilityRegistry, InProcBus, Supervisor};
use semver::{Version, VersionReq};
use serde_json::json;
use tokio::sync::mpsc;

// ---- Test agents ----------------------------------------------------------

struct Echo {
    id: AgentId,
    caps: Vec<Capability>,
}

impl Echo {
    fn new() -> Self {
        Self {
            id: AgentId::new("echo-1"),
            caps: vec![Capability::read_only("test.echo", Version::new(1, 0, 0))],
        }
    }
}

#[async_trait]
impl Agent for Echo {
    fn id(&self) -> &AgentId {
        &self.id
    }
    fn capabilities(&self) -> &[Capability] {
        &self.caps
    }
    async fn handle(&self, env: AgentEnvelope, ctx: AgentContext) -> Result<(), CoreError> {
        let reply = AgentEnvelope::reply_to(&env, ctx.agent_id.clone(), env.body.clone());
        ctx.bus.send(reply).await
    }
}

/// Ping sends one envelope to "echo" by capability and forwards the reply
/// (correlation id of the original) to a `tokio::mpsc::Sender` so the test
/// can observe completion.
struct Ping {
    id: AgentId,
    caps: Vec<Capability>,
    observed_tx: mpsc::Sender<AgentEnvelope>,
}

impl Ping {
    fn new(observed_tx: mpsc::Sender<AgentEnvelope>) -> Self {
        Self {
            id: AgentId::new("ping-1"),
            caps: vec![Capability::read_only("test.ping", Version::new(1, 0, 0))],
            observed_tx,
        }
    }
}

#[async_trait]
impl Agent for Ping {
    fn id(&self) -> &AgentId {
        &self.id
    }
    fn capabilities(&self) -> &[Capability] {
        &self.caps
    }
    async fn handle(&self, env: AgentEnvelope, _ctx: AgentContext) -> Result<(), CoreError> {
        let _ = self.observed_tx.send(env).await;
        Ok(())
    }
}

/// PanicOnce returns success on the first envelope, panics on the second,
/// then succeeds again after restart. Used to verify supervisor restarts.
struct PanicOnce {
    id: AgentId,
    caps: Vec<Capability>,
    invocation_count: Arc<AtomicU32>,
    observed_tx: mpsc::Sender<u32>,
}

impl PanicOnce {
    fn new(observed_tx: mpsc::Sender<u32>) -> (Self, Arc<AtomicU32>) {
        let invocation_count = Arc::new(AtomicU32::new(0));
        (
            Self {
                id: AgentId::new("panic-once"),
                caps: vec![Capability::read_only(
                    "test.panic_once",
                    Version::new(1, 0, 0),
                )],
                invocation_count: invocation_count.clone(),
                observed_tx,
            },
            invocation_count,
        )
    }
}

#[async_trait]
impl Agent for PanicOnce {
    fn id(&self) -> &AgentId {
        &self.id
    }
    fn capabilities(&self) -> &[Capability] {
        &self.caps
    }
    async fn handle(&self, _env: AgentEnvelope, _ctx: AgentContext) -> Result<(), CoreError> {
        let n = self.invocation_count.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.observed_tx.send(n).await;
        if n == 2 {
            panic!("planned panic on 2nd invocation");
        }
        Ok(())
    }
}

// ---- Tests ---------------------------------------------------------------

#[tokio::test]
async fn roundtrip_direct_addressing() {
    let bus = InProcBus::new();
    let registry = CapabilityRegistry::new();
    let supervisor = Supervisor::new(bus.clone(), registry);

    supervisor.spawn(Arc::new(Echo::new())).await.unwrap();
    let (tx, mut rx) = mpsc::channel(8);
    supervisor.spawn(Arc::new(Ping::new(tx))).await.unwrap();

    // Give the agents a moment to subscribe.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let req = AgentEnvelope::new(
        AgentId::new("ping-1"),
        Recipient::Direct(AgentId::new("echo-1")),
        json!({"msg": "hello"}),
    );
    let correlation_id = req.correlation_id;
    BusHandle::send(&bus, req).await.unwrap();

    let reply = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("reply did not arrive")
        .expect("channel closed");

    assert_eq!(reply.correlation_id, correlation_id);
    assert_eq!(reply.body["msg"], "hello");
}

#[tokio::test]
async fn roundtrip_by_capability_addressing() {
    let bus = InProcBus::new();
    let registry = CapabilityRegistry::new();
    let supervisor = Supervisor::new(bus.clone(), registry.clone());

    supervisor.spawn(Arc::new(Echo::new())).await.unwrap();
    let (tx, mut rx) = mpsc::channel(8);
    supervisor.spawn(Arc::new(Ping::new(tx))).await.unwrap();

    // Verify the registry sees both capabilities.
    let req: VersionReq = "^1".parse().unwrap();
    let matches = registry.find_by_capability("test.echo", &req).await;
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].agent_id.as_str(), "echo-1");

    tokio::time::sleep(Duration::from_millis(50)).await;

    let req = AgentEnvelope::new(
        AgentId::new("ping-1"),
        Recipient::ByCapability {
            name: "test.echo".into(),
            version_req: "^1".parse().unwrap(),
        },
        json!({"by": "capability"}),
    );
    let correlation_id = req.correlation_id;
    BusHandle::send(&bus, req).await.unwrap();

    let reply = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("reply did not arrive")
        .expect("channel closed");
    assert_eq!(reply.correlation_id, correlation_id);
    assert_eq!(reply.body["by"], "capability");
}

#[tokio::test]
async fn duplicate_spawn_is_rejected() {
    let bus = InProcBus::new();
    let registry = CapabilityRegistry::new();
    let supervisor = Supervisor::new(bus, registry);

    supervisor.spawn(Arc::new(Echo::new())).await.unwrap();
    let err = supervisor.spawn(Arc::new(Echo::new())).await.unwrap_err();
    assert!(matches!(
        err,
        daimon_runtime::RuntimeError::DuplicateAgent(_)
    ));
}

#[tokio::test]
async fn supervisor_restarts_after_panic() {
    let bus = InProcBus::new();
    let registry = CapabilityRegistry::new();
    let supervisor = Supervisor::new(bus.clone(), registry);

    let (tx, mut rx) = mpsc::channel(8);
    let (agent, count) = PanicOnce::new(tx);
    supervisor.spawn(Arc::new(agent)).await.unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send 3 envelopes spaced out enough for restart to settle between them.
    for _ in 0..3 {
        let env = AgentEnvelope::new(
            AgentId::new("driver"),
            Recipient::Direct(AgentId::new("panic-once")),
            json!({}),
        );
        BusHandle::send(&bus, env).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // We expect to see invocation numbers 1, 2, 3 — meaning the agent was
    // restarted after the panic on invocation 2 and continued serving.
    let mut invocations = vec![];
    while let Ok(Some(n)) = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
        invocations.push(n);
    }
    assert!(
        invocations.contains(&1) && invocations.contains(&2) && invocations.contains(&3),
        "expected to see invocations 1, 2, 3 after restart; saw {invocations:?}"
    );
    assert!(count.load(Ordering::SeqCst) >= 3);
}

#[tokio::test]
async fn stop_unregisters_agent() {
    let bus = InProcBus::new();
    let registry = CapabilityRegistry::new();
    let supervisor = Supervisor::new(bus, registry.clone());

    supervisor.spawn(Arc::new(Echo::new())).await.unwrap();
    assert!(registry.get(&AgentId::new("echo-1")).await.is_some());

    supervisor.stop(&AgentId::new("echo-1")).await.unwrap();
    assert!(registry.get(&AgentId::new("echo-1")).await.is_none());
}
