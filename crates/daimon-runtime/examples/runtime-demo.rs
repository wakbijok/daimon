//! Phase 1 smoke demo for the daimon multi-agent runtime.
//!
//! Spawns two trivial agents (echo + ping), prints the capability registry,
//! sends one roundtrip envelope, then exits. Use this as the first sanity
//! check after touching daimon-core or daimon-runtime.
//!
//! Run with: `cargo run -p daimon-runtime --example runtime-demo`

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use daimon_core::{
    Agent, AgentContext, AgentEnvelope, AgentId, BusHandle, Capability, CoreError, Recipient,
};
use daimon_runtime::{CapabilityRegistry, InProcBus, Supervisor};
use semver::Version;
use serde_json::json;
use tokio::sync::mpsc;

struct Echo {
    id: AgentId,
    caps: Vec<Capability>,
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

struct Ping {
    id: AgentId,
    caps: Vec<Capability>,
    observed_tx: mpsc::Sender<AgentEnvelope>,
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().init();

    let bus = InProcBus::new();
    let registry = CapabilityRegistry::new();
    let supervisor = Supervisor::new(bus.clone(), registry.clone());

    supervisor
        .spawn(Arc::new(Echo {
            id: AgentId::new("echo-1"),
            caps: vec![Capability::read_only(
                "test.echo",
                Version::new(1, 0, 0),
            )],
        }))
        .await?;

    let (tx, mut rx) = mpsc::channel(8);
    supervisor
        .spawn(Arc::new(Ping {
            id: AgentId::new("ping-1"),
            caps: vec![Capability::read_only(
                "test.ping",
                Version::new(1, 0, 0),
            )],
            observed_tx: tx,
        }))
        .await?;

    // Allow subscriptions to settle.
    tokio::time::sleep(Duration::from_millis(50)).await;

    println!("Registered agents:");
    for entry in registry.all().await {
        for cap in &entry.capabilities {
            println!("  {} -> {} v{}", entry.agent_id, cap.name, cap.version);
        }
    }

    let req = AgentEnvelope::new(
        AgentId::new("ping-1"),
        Recipient::ByCapability {
            name: "test.echo".into(),
            version_req: "^1".parse()?,
        },
        json!({"hello": "world"}),
    );
    let cid = req.correlation_id;
    println!("\nSending envelope correlation_id={cid} to test.echo ^1");
    BusHandle::send(&bus, req).await?;

    let reply = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await?
        .ok_or("no reply received")?;
    println!(
        "Reply correlation_id={} body={}",
        reply.correlation_id, reply.body
    );

    supervisor.shutdown().await;
    Ok(())
}
