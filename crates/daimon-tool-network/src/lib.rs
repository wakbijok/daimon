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
            // Phase 8 — write capabilities. Both go through Guard's
            // policy + approval-inbox pre-flight in broker.execute (D19/
            // Phase 5). The compensating pair is D18-saga material:
            // `firewall_add_drop_rule` is rolled back by
            // `firewall_remove_rule`, which is itself irreversible (a
            // removed rule is gone). Phase 6.1 wires the auto-rollback.
            Capability {
                name: "network.routeros.firewall_add_drop_rule".into(),
                version: Version::new(1, 0, 0),
                description: Some(
                    "Append a drop-action rule to `/ip firewall filter`. \
                     Params: src_address (CIDR), dst_address (CIDR or address-list name), \
                     in_interface (optional), comment (optional)."
                        .into(),
                ),
                schema: Some(serde_json::json!({
                    "type": "object",
                    "required": ["dst_address"],
                    "properties": {
                        "src_address": { "type": "string" },
                        "dst_address": { "type": "string" },
                        "in_interface": { "type": "string" },
                        "comment": { "type": "string" }
                    }
                })),
                compensating: Some(daimon_core::CompensatingCapability {
                    name: "network.routeros.firewall_remove_rule".into(),
                    version_req: None,
                }),
                irreversible: false,
            },
            // Phase 8 second vertical — SSH user key rotation. RouterOS
            // syntax: `/user ssh-keys import file=<basename> user=<user>`.
            // The new key has to be uploaded out-of-band first (SFTP or
            // /file print upload); this capability runs the import.
            // Compensating capability is the removal of the new key —
            // operator runs this if the new key is revealed compromised.
            Capability {
                name: "network.routeros.user_ssh_key_import".into(),
                version: Version::new(1, 0, 0),
                description: Some(
                    "Import a public SSH key into RouterOS `/user ssh-keys`. \
                     Params: user (RouterOS user name), file (basename of \
                     the uploaded .pub key on the device)."
                        .into(),
                ),
                schema: Some(serde_json::json!({
                    "type": "object",
                    "required": ["user", "file"],
                    "properties": {
                        "user": { "type": "string" },
                        "file": { "type": "string" }
                    }
                })),
                compensating: Some(daimon_core::CompensatingCapability {
                    name: "network.routeros.user_ssh_key_remove".into(),
                    version_req: None,
                }),
                irreversible: false,
            },
            Capability {
                name: "network.routeros.user_ssh_key_remove".into(),
                version: Version::new(1, 0, 0),
                description: Some(
                    "Remove an SSH key entry from `/user ssh-keys`. \
                     Params: number (the row number from ssh-keys print)."
                        .into(),
                ),
                schema: Some(serde_json::json!({
                    "type": "object",
                    "required": ["number"],
                    "properties": { "number": { "type": "string" } }
                })),
                compensating: None,
                irreversible: true,
            },
            Capability {
                name: "network.routeros.firewall_remove_rule".into(),
                version: Version::new(1, 0, 0),
                description: Some(
                    "Remove a rule from `/ip firewall filter` by id or comment match."
                        .into(),
                ),
                schema: Some(serde_json::json!({
                    "type": "object",
                    "required": ["match"],
                    "properties": {
                        "match": { "type": "string" }
                    }
                })),
                compensating: None,
                // A removed rule can't be auto-restored — operator
                // confirmation required on every invocation.
                irreversible: true,
            },
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
        let command = build_command(&req.capability, req.params.as_ref())?;
        let target = InvTargetRef::parse(&req.target_ref)
            .map_err(|e| NetworkAgentError::BadTarget(format!("{e}")))?;
        let timeout_secs = req.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS).max(1);
        let is_read_only = is_read_only_capability(&req.capability);

        let exec_req = ExecRequest::new(
            self.actor_id.clone(),
            target,
            Op::ShellCommand {
                command: command.clone(),
                timeout_secs,
            },
        )
        .with_capability(&req.capability, is_read_only);

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
                command,
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

fn is_read_only_capability(capability: &str) -> bool {
    matches!(
        capability,
        "network.routeros.system_info"
            | "network.routeros.interface_list"
            | "network.routeros.ip_addresses"
            | "network.routeros.firewall_filter_list"
    )
}

/// Build the RouterOS CLI command for a capability. Read capabilities take
/// no params and resolve to a fixed string; write capabilities interpolate
/// from `params` against the schema declared in the capability registry.
///
/// Failures: unknown capability, missing required params, or params that
/// would inject shell control characters (validated client-side; the SSH
/// transport on the server side also escapes, but we double-check here).
fn build_command(
    capability: &str,
    params: Option<&serde_json::Value>,
) -> Result<String, NetworkAgentError> {
    match capability {
        "network.routeros.system_info" => Ok("/system identity print".into()),
        "network.routeros.interface_list" => Ok("/interface print".into()),
        "network.routeros.ip_addresses" => Ok("/ip address print".into()),
        "network.routeros.firewall_filter_list" => Ok("/ip firewall filter print".into()),
        "network.routeros.firewall_add_drop_rule" => {
            let p = params.ok_or_else(|| {
                NetworkAgentError::UnknownCapability(
                    "firewall_add_drop_rule requires params".into(),
                )
            })?;
            let dst = p
                .get("dst_address")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    NetworkAgentError::UnknownCapability(
                        "firewall_add_drop_rule.dst_address is required".into(),
                    )
                })?;
            reject_shell_metachars(dst)?;
            let mut cmd = format!(
                "/ip firewall filter add chain=forward action=drop dst-address={dst}"
            );
            if let Some(src) = p.get("src_address").and_then(|v| v.as_str()) {
                reject_shell_metachars(src)?;
                cmd.push_str(&format!(" src-address={src}"));
            }
            if let Some(iface) = p.get("in_interface").and_then(|v| v.as_str()) {
                reject_shell_metachars(iface)?;
                cmd.push_str(&format!(" in-interface={iface}"));
            }
            if let Some(comment) = p.get("comment").and_then(|v| v.as_str()) {
                reject_shell_metachars(comment)?;
                cmd.push_str(&format!(" comment=\"{comment}\""));
            }
            Ok(cmd)
        }
        "network.routeros.firewall_remove_rule" => {
            let p = params.ok_or_else(|| {
                NetworkAgentError::UnknownCapability(
                    "firewall_remove_rule requires params".into(),
                )
            })?;
            let m = p.get("match").and_then(|v| v.as_str()).ok_or_else(|| {
                NetworkAgentError::UnknownCapability(
                    "firewall_remove_rule.match is required".into(),
                )
            })?;
            reject_shell_metachars(m)?;
            Ok(format!("/ip firewall filter remove [find {m}]"))
        }
        "network.routeros.user_ssh_key_import" => {
            let p = params.ok_or_else(|| {
                NetworkAgentError::UnknownCapability(
                    "user_ssh_key_import requires params".into(),
                )
            })?;
            let user = p.get("user").and_then(|v| v.as_str()).ok_or_else(|| {
                NetworkAgentError::UnknownCapability(
                    "user_ssh_key_import.user is required".into(),
                )
            })?;
            let file = p.get("file").and_then(|v| v.as_str()).ok_or_else(|| {
                NetworkAgentError::UnknownCapability(
                    "user_ssh_key_import.file is required".into(),
                )
            })?;
            reject_shell_metachars(user)?;
            reject_shell_metachars(file)?;
            Ok(format!("/user ssh-keys import user={user} public-key-file={file}"))
        }
        "network.routeros.user_ssh_key_remove" => {
            let p = params.ok_or_else(|| {
                NetworkAgentError::UnknownCapability(
                    "user_ssh_key_remove requires params".into(),
                )
            })?;
            let n = p.get("number").and_then(|v| v.as_str()).ok_or_else(|| {
                NetworkAgentError::UnknownCapability(
                    "user_ssh_key_remove.number is required".into(),
                )
            })?;
            reject_shell_metachars(n)?;
            Ok(format!("/user ssh-keys remove {n}"))
        }
        other => Err(NetworkAgentError::UnknownCapability(other.to_string())),
    }
}

/// Reject params containing shell metachars / quotes — RouterOS's CLI
/// parser tolerates more than POSIX sh, but daimon's brokered SSH
/// transport pipes the line as a single command and any unescaped quote
/// or semicolon would break the boundary. Banking-grade input validation:
/// allow `[A-Za-z0-9._:/!@\-]` plus dot, slash, dash. Anything else is
/// rejected and the operator must use a more specific capability with
/// stronger schema.
fn reject_shell_metachars(s: &str) -> Result<(), NetworkAgentError> {
    if s.is_empty() {
        return Err(NetworkAgentError::UnknownCapability("empty param".into()));
    }
    for c in s.chars() {
        let ok = c.is_ascii_alphanumeric()
            || matches!(c, '.' | '/' | '-' | '_' | ':' | '!' | '@');
        if !ok {
            return Err(NetworkAgentError::UnknownCapability(format!(
                "param contains disallowed char `{c}` (allow [A-Za-z0-9._:/!@-])"
            )));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRequest {
    pub capability: String,
    pub target_ref: String,
    #[serde(default)]
    pub timeout_secs: Option<u32>,
    /// Write-capability params (validated by `build_command`). `None` for
    /// read capabilities.
    #[serde(default)]
    pub params: Option<serde_json::Value>,
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
