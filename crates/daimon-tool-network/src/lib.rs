//! Network worker agent (Phase 4 D2).
//!
//! First Worker Agent in daimon. Read-only RouterOS-CLI capabilities over
//! SSH via the broker. Per D21, this crate depends on `daimon-broker` (the
//! public surface) and NOT on vault/inventory/transport directly.
//!
//! Capabilities (Phase 4 — read-only):
//! - `network.routeros.system_info` (v1.0.0) — `/system identity print`
//! - `network.routeros.interface_list` (v1.0.0) — `/interface print`
//! - `network.routeros.ip_addresses` (v1.0.0) — `/ip address print`
//! - `network.routeros.firewall_filter_list` (v1.0.0) — `/ip firewall filter print`
//!
//! Phase 5 adds write capabilities (e.g. `firewall.filter_add`) all
//! Guard-gated. Phase 6 sees these capabilities used by the orchestrator.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use daimon_broker::{
    Broker, BrokerError, ExecRequest, Op, OpResult, TargetRef as InvTargetRef,
};
use daimon_core::{
    Agent, AgentContext, AgentEnvelope, AgentId, Capability, CoreError, Recipient,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, error, instrument};

const DEFAULT_TIMEOUT_SECS: u32 = 30;

/// Network agent.
///
/// Holds an `Arc<Broker>` for credential-safe SSH dispatch. The agent
/// itself stores no per-target state — every invocation looks up the
/// target via the broker's inventory.
pub struct NetworkAgent {
    id: AgentId,
    broker: Arc<Broker>,
    capabilities: Vec<Capability>,
    actor_id: String,
    timeout: Duration,
}

impl NetworkAgent {
    /// Construct a network agent. `actor_id` is the audit-event identity
    /// used when this agent invokes the broker (typically `"agent:network"`
    /// or `"agent:network:<instance>"`).
    pub fn new(id: AgentId, broker: Arc<Broker>, actor_id: impl Into<String>) -> Self {
        let capabilities = vec![
            Capability::read_only("network.routeros.system_info", Version::new(1, 0, 0)),
            Capability::read_only("network.routeros.interface_list", Version::new(1, 0, 0)),
            Capability::read_only("network.routeros.ip_addresses", Version::new(1, 0, 0)),
            Capability::read_only(
                "network.routeros.firewall_filter_list",
                Version::new(1, 0, 0),
            ),
        ];
        Self {
            id,
            broker,
            capabilities,
            actor_id: actor_id.into(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS as u64),
        }
    }
}

#[async_trait]
impl Agent for NetworkAgent {
    fn id(&self) -> &AgentId {
        &self.id
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    #[instrument(skip(self, ctx), fields(agent = %self.id))]
    async fn handle(&self, env: AgentEnvelope, ctx: AgentContext) -> Result<(), CoreError> {
        // Each incoming envelope carries a JSON body shaped as
        // `NetworkRequest`. Dispatch by `capability` field.
        let req: NetworkRequest = match serde_json::from_value(env.body.clone()) {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, "invalid NetworkRequest body");
                let resp = NetworkResponse::error(format!("invalid request: {e}"));
                let reply = AgentEnvelope::reply_to(
                    &env,
                    self.id.clone(),
                    serde_json::to_value(resp).unwrap_or_default(),
                );
                return ctx.bus.send(reply).await;
            }
        };

        debug!(capability = %req.capability, target = %req.target_ref, "dispatch");
        let result = self.invoke(&req).await;
        let resp = match result {
            Ok(out) => NetworkResponse::ok(out),
            Err(e) => {
                error!(error = %e, "network capability failed");
                NetworkResponse::error(e.to_string())
            }
        };
        let reply = AgentEnvelope::reply_to(
            &env,
            self.id.clone(),
            serde_json::to_value(resp).unwrap_or_default(),
        );
        ctx.bus.send(reply).await
    }
}

impl NetworkAgent {
    async fn invoke(&self, req: &NetworkRequest) -> Result<NetworkOutput, NetworkAgentError> {
        let command = command_for(&req.capability)?;
        let target = InvTargetRef::parse(&req.target_ref)
            .map_err(|e| NetworkAgentError::BadTarget(format!("{e}")))?;
        let timeout_secs = req.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS).max(1);

        let exec_req = ExecRequest::new(
            self.actor_id.clone(),
            target,
            Op::ShellCommand {
                command: command.to_string(),
                timeout_secs,
            },
        )
        .with_capability(&req.capability, true /* Phase 4 capabilities are all read-only */);

        let op_result = self
            .broker
            .execute(exec_req)
            .await
            .map_err(NetworkAgentError::Broker)?;

        match op_result {
            OpResult::ShellCommand {
                stdout,
                stderr,
                exit_status,
            } => Ok(NetworkOutput {
                command: command.to_string(),
                stdout,
                stderr,
                exit_status,
            }),
            other => Err(NetworkAgentError::WrongOpResult(format!("{other:?}"))),
        }
    }

    /// Surface helper: agents that don't have an incoming envelope (e.g.
    /// orchestrator picking up a tool-use directly) call this synchronously.
    pub async fn run(&self, req: NetworkRequest) -> Result<NetworkOutput, NetworkAgentError> {
        self.invoke(&req).await
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// Map a capability name to the corresponding read-only RouterOS CLI command.
fn command_for(capability: &str) -> Result<&'static str, NetworkAgentError> {
    match capability {
        "network.routeros.system_info" => Ok("/system identity print"),
        "network.routeros.interface_list" => Ok("/interface print"),
        "network.routeros.ip_addresses" => Ok("/ip address print"),
        "network.routeros.firewall_filter_list" => Ok("/ip firewall filter print"),
        other => Err(NetworkAgentError::UnknownCapability(other.to_string())),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRequest {
    pub capability: String,
    pub target_ref: String,
    #[serde(default)]
    pub timeout_secs: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkResponse {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<NetworkOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl NetworkResponse {
    fn ok(output: NetworkOutput) -> Self {
        Self {
            success: true,
            output: Some(output),
            error: None,
        }
    }
    fn error(msg: String) -> Self {
        Self {
            success: false,
            output: None,
            error: Some(msg),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkOutput {
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_status: i32,
}

#[derive(Debug, Error)]
pub enum NetworkAgentError {
    #[error("unknown capability: {0}")]
    UnknownCapability(String),
    #[error("invalid target_ref: {0}")]
    BadTarget(String),
    #[error("broker: {0}")]
    Broker(BrokerError),
    #[error("unexpected OpResult variant: {0}")]
    WrongOpResult(String),
}

// Suppress unused warning on Recipient — re-exported so callers can build
// envelopes addressed to this agent by capability.
#[allow(dead_code)]
fn _hint(_: Recipient) {}
