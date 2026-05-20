use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use daimon_core::{Agent, AgentContext, AgentEnvelope, AgentId, Recipient};
use tokio::sync::{Mutex, RwLock, broadcast};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::bus::{AgentBus, InProcBus};
use crate::error::RuntimeError;
use crate::registry::CapabilityRegistry;

/// Tunables for the supervisor.
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    /// Initial restart delay after a crash.
    pub restart_initial_delay: Duration,
    /// Maximum restart delay after exponential backoff.
    pub restart_max_delay: Duration,
    /// If more than this many restarts happen within `restart_window`, give up.
    pub restart_max_in_window: usize,
    pub restart_window: Duration,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            restart_initial_delay: Duration::from_millis(100),
            restart_max_delay: Duration::from_secs(30),
            restart_max_in_window: 5,
            restart_window: Duration::from_secs(60),
        }
    }
}

/// Owns the agent lifecycle.
///
/// `spawn` registers an agent with the bus + registry and starts a tokio task
/// that drives its `handle` calls. If the task exits (clean return, panic, or
/// error), the supervisor restarts it with exponential backoff up to a
/// configured limit.
pub struct Supervisor {
    bus: InProcBus,
    registry: CapabilityRegistry,
    config: SupervisorConfig,
    /// Tracks per-agent join handles so the supervisor can stop them.
    handles: Arc<Mutex<HashMap<AgentId, JoinHandle<()>>>>,
    /// Holds Arc<dyn Agent> for each spawned agent so restarts can respawn the
    /// same instance.
    agents: Arc<RwLock<HashMap<AgentId, Arc<dyn Agent>>>>,
}

impl Supervisor {
    pub fn new(bus: InProcBus, registry: CapabilityRegistry) -> Self {
        Self::with_config(bus, registry, SupervisorConfig::default())
    }

    pub fn with_config(
        bus: InProcBus,
        registry: CapabilityRegistry,
        config: SupervisorConfig,
    ) -> Self {
        Self {
            bus,
            registry,
            config,
            handles: Arc::new(Mutex::new(HashMap::new())),
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn bus(&self) -> &InProcBus {
        &self.bus
    }

    pub fn registry(&self) -> &CapabilityRegistry {
        &self.registry
    }

    /// Spawn an agent. Registers its capabilities, subscribes a receiver,
    /// starts a tokio task that drives the handler loop with restart-on-panic.
    pub async fn spawn(&self, agent: Arc<dyn Agent>) -> Result<(), RuntimeError> {
        let agent_id = agent.id().clone();

        {
            let agents = self.agents.read().await;
            if agents.contains_key(&agent_id) {
                return Err(RuntimeError::DuplicateAgent(agent_id.to_string()));
            }
        }

        self.registry
            .register(agent_id.clone(), agent.capabilities().to_vec())
            .await;

        self.agents
            .write()
            .await
            .insert(agent_id.clone(), agent.clone());

        let handle = self.spawn_runner(agent_id.clone(), agent).await;
        self.handles.lock().await.insert(agent_id, handle);
        Ok(())
    }

    async fn spawn_runner(&self, agent_id: AgentId, agent: Arc<dyn Agent>) -> JoinHandle<()> {
        let bus = self.bus.clone();
        let registry = self.registry.clone();
        let config = self.config.clone();
        let handles = self.handles.clone();
        let agents = self.agents.clone();

        tokio::spawn(async move {
            let mut delay = config.restart_initial_delay;
            let mut restarts = 0usize;
            let mut window_start = std::time::Instant::now();

            loop {
                let agent_clone = agent.clone();
                let bus_clone = bus.clone();
                let id_clone = agent_id.clone();

                let outcome = tokio::spawn(async move {
                    run_agent_loop(agent_clone, bus_clone, id_clone).await
                })
                .await;

                match outcome {
                    Ok(Ok(())) => {
                        info!(agent = %agent_id, "agent exited cleanly");
                        break;
                    }
                    Ok(Err(e)) => {
                        warn!(agent = %agent_id, error = %e, "agent returned error");
                    }
                    Err(join_err) => {
                        if join_err.is_panic() {
                            error!(agent = %agent_id, "agent panicked");
                        } else {
                            error!(agent = %agent_id, error = ?join_err, "agent task aborted");
                            break;
                        }
                    }
                }

                if window_start.elapsed() > config.restart_window {
                    window_start = std::time::Instant::now();
                    restarts = 0;
                }
                restarts += 1;
                if restarts > config.restart_max_in_window {
                    error!(
                        agent = %agent_id,
                        restarts,
                        "agent exceeded restart budget — giving up"
                    );
                    break;
                }

                info!(agent = %agent_id, ?delay, "restarting agent");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(config.restart_max_delay);
            }

            registry.unregister(&agent_id).await;
            handles.lock().await.remove(&agent_id);
            agents.write().await.remove(&agent_id);
        })
    }

    /// Stop an agent. Aborts its task and removes it from the registry.
    pub async fn stop(&self, agent_id: &AgentId) -> Result<(), RuntimeError> {
        if let Some(handle) = self.handles.lock().await.remove(agent_id) {
            handle.abort();
        }
        self.registry.unregister(agent_id).await;
        self.agents.write().await.remove(agent_id);
        Ok(())
    }

    /// Stop all agents.
    pub async fn shutdown(&self) {
        let agent_ids: Vec<AgentId> = self.agents.read().await.keys().cloned().collect();
        for id in agent_ids {
            let _ = self.stop(&id).await;
        }
    }
}

/// Drives a single agent's handler loop. Receives envelopes, filters by
/// `to` field, dispatches to `agent.handle`.
async fn run_agent_loop(
    agent: Arc<dyn Agent>,
    bus: InProcBus,
    agent_id: AgentId,
) -> Result<(), RuntimeError> {
    let ctx = AgentContext::new(agent_id.clone(), bus.handle());
    let mut rx = bus.subscribe_raw();

    loop {
        match rx.recv().await {
            Ok(env) => {
                if !envelope_addressed_to(&env, &agent_id, agent.capabilities()) {
                    continue;
                }
                let ctx_clone = ctx.clone();
                if let Err(e) = agent.handle(env, ctx_clone).await {
                    warn!(agent = %agent_id, error = %e, "handler returned error");
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!(agent = %agent_id, lost = n, "bus receiver lagged");
            }
            Err(broadcast::error::RecvError::Closed) => {
                info!(agent = %agent_id, "bus closed");
                break;
            }
        }
    }
    Ok(())
}

fn envelope_addressed_to(
    env: &AgentEnvelope,
    me: &AgentId,
    my_capabilities: &[daimon_core::Capability],
) -> bool {
    match &env.to {
        Recipient::Direct(id) => id == me,
        Recipient::ByCapability { name, version_req } => my_capabilities
            .iter()
            .any(|cap| cap.name == *name && version_req.matches(&cap.version)),
    }
}
