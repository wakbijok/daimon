//! Reference `Driver`: RouterOS firewall/network over SSH (P2 commit 3).
//!
//! This crate is the first concrete implementation of the [`daimon_driver::Driver`]
//! trait. It is a refactor of the Phase-4/8 `NetworkAgent` (`daimon-tool-network`)
//! into the class-uniform driver seam:
//!
//! - the RouterOS capability catalog (`network.routeros.*`) is exposed via
//!   [`Driver::capabilities`];
//! - the four read caps (`system_info` / `interface_list` / `ip_addresses` /
//!   `firewall_filter_list`) back [`Driver::read_state`] and [`Driver::describe`];
//! - the write caps (`firewall_add_drop_rule` / `firewall_remove_rule` /
//!   `user_ssh_key_import` / `user_ssh_key_remove`) back [`Driver::remediate`];
//! - [`Driver::diagnose`] reads the filter table and surfaces `Finding`s whose
//!   `suggested_capability` points at `firewall_remove_rule` — it NEVER decides
//!   the fix, it only surfaces candidates for the LLM/orchestrator.
//!
//! # The credential boundary (D21)
//!
//! Per D21 this crate depends on `daimon-core`, `daimon-broker` and
//! `daimon-driver` only — NOT on vault/inventory/transport. Every verb builds an
//! `Op::ShellCommand` via [`build_command`] and dispatches it through
//! `Broker::execute` with the resolved `Capability` attached as `capability_meta`
//! (the server-side read-only/irreversible/compensating authority, H6/H7). The
//! driver never resolves a credential or opens a transport.
//!
//! # Injection chokepoint (FR-CON-12)
//!
//! The old local `reject_shell_metachars` is gone. Every write param is now
//! validated with [`daimon_driver::param::validate`] against a declared
//! [`ParamClass`] BEFORE it is substituted into a command — the single, shared
//! injection chokepoint used by both the code drivers and the generic
//! `ConnectorDriver`.
//!
//! # Bus adapter (FR-HAR-17)
//!
//! [`RouterOsDriver`] also implements `daimon_core::Agent` so the supervisor can
//! spawn it on the bus. The adapter decodes an incoming [`NetworkRequest`],
//! carries **the caller identity from `env.from`** as the ExecRequest actor (so
//! audit/approval records the real requester, NOT a static agent id), calls the
//! matching `Driver` verb, and replies with a [`NetworkResponse`].

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use daimon_broker::{Broker, BrokerError, ExecRequest, Op, OpResult, TargetRef as InvTargetRef};
use daimon_core::{Agent, AgentContext, AgentEnvelope, AgentId, Capability, CoreError, Recipient};
use daimon_driver::param::{self, ParamClass};
use daimon_driver::{
    Driver, DriverError, DriverResult, Finding, Receipt, Severity, StateDoc, TargetClass,
    TargetShape,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, error, instrument};

const DEFAULT_TIMEOUT_SECS: u32 = 30;

/// RouterOS firewall/network driver.
///
/// Holds an `Arc<Broker>` for credential-safe SSH dispatch. Stores no
/// per-target state — every verb looks the target up via the broker's inventory.
pub struct RouterOsDriver {
    id: AgentId,
    broker: Arc<Broker>,
    capabilities: Vec<Capability>,
    /// Fallback audit identity used ONLY when a caller does not supply one (the
    /// synchronous `remediate`/`read_state` entrypoints). The bus adapter always
    /// overrides this with `env.from` (FR-HAR-17).
    actor_id: String,
    timeout: Duration,
}

impl RouterOsDriver {
    /// Construct a RouterOS driver. `actor_id` is the fallback audit identity
    /// used by the synchronous driver verbs (`agent:network` by convention). The
    /// bus adapter carries the real caller from `env.from` instead.
    pub fn new(id: AgentId, broker: Arc<Broker>, actor_id: impl Into<String>) -> Self {
        Self {
            id,
            broker,
            capabilities: build_capabilities(),
            actor_id: actor_id.into(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS as u64),
        }
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The core dispatch: build the RouterOS command, resolve the target,
    /// attach the capability descriptor, and submit to the broker — the SAME
    /// security path the old `NetworkAgent::invoke` used. `actor_id` is the
    /// audit identity (from `env.from` on the bus path).
    async fn dispatch(
        &self,
        actor_id: &str,
        capability: &str,
        target_ref: &str,
        params: Option<&serde_json::Value>,
        timeout_secs: u32,
    ) -> Result<NetworkOutput, RouterOsError> {
        let command = build_command(capability, params)?;
        let exec_req = build_exec_request(
            actor_id,
            capability,
            target_ref,
            &command,
            timeout_secs,
            &self.capabilities,
        )?;

        let op_result = self
            .broker
            .execute(exec_req)
            .await
            .map_err(RouterOsError::Broker)?;

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
            other => Err(RouterOsError::WrongOpResult(format!("{other:?}"))),
        }
    }
}

/// Assemble the `ExecRequest` for a rendered RouterOS command. Pure (no broker
/// I/O) so the caller-identity plumbing is unit-testable: `actor_id` — which the
/// bus adapter sources from `env.from` (FR-HAR-17) — lands verbatim on
/// `ExecRequest.actor_id`, and the resolved `Capability` is attached as
/// `capability_meta` so the broker derives the read-only disposition server-side
/// (H6/H7). An unknown capability name carries no descriptor -> broker treats it
/// as a write (fail-closed).
fn build_exec_request(
    actor_id: &str,
    capability: &str,
    target_ref: &str,
    command: &str,
    timeout_secs: u32,
    capabilities: &[Capability],
) -> Result<ExecRequest, RouterOsError> {
    let target =
        InvTargetRef::parse(target_ref).map_err(|e| RouterOsError::BadTarget(format!("{e}")))?;
    let exec_req = ExecRequest::new(
        actor_id.to_string(),
        target,
        Op::ShellCommand {
            command: command.to_string(),
            timeout_secs: timeout_secs.max(1),
        },
    );
    Ok(match capabilities.iter().find(|c| c.name == capability) {
        Some(cap) => exec_req.with_capability_meta(cap.clone()),
        None => exec_req.with_capability(capability, false),
    })
}

// -------------------------------------------------------------------------
// Driver trait impl
// -------------------------------------------------------------------------

#[async_trait]
impl Driver for RouterOsDriver {
    /// RouterOS filter rules are the firewall surface of a router; the write
    /// caps mutate `/ip firewall filter` and `/user ssh-keys`, so this driver's
    /// class is `Firewall` (matching `parse_class("network.routeros.*")` in
    /// daimon-driver, which maps the `routeros` subsystem segment to Firewall).
    fn class(&self) -> TargetClass {
        TargetClass::Firewall
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    /// READ — identity of the target (`/system identity print`).
    async fn describe(&self, target: &str) -> DriverResult<TargetShape> {
        let out = self
            .dispatch(
                &self.actor_id,
                "network.routeros.system_info",
                target,
                None,
                DEFAULT_TIMEOUT_SECS,
            )
            .await
            .map_err(driver_err)?;
        Ok(TargetShape {
            class: TargetClass::Firewall,
            identity: serde_json::json!({ "system_identity": out.stdout.trim() }),
            summary: format!("RouterOS device {target}"),
        })
    }

    /// READ — a live typed snapshot for `selector`. Backs the four read caps:
    /// `{"table":"firewall.filter"}`, `{"table":"interface"}`,
    /// `{"table":"ip.address"}`, or `{"table":"system"}` (default: system).
    async fn read_state(
        &self,
        target: &str,
        selector: serde_json::Value,
    ) -> DriverResult<StateDoc> {
        let table = selector
            .get("table")
            .and_then(|v| v.as_str())
            .unwrap_or("system");
        let capability = match table {
            "system" | "system_info" => "network.routeros.system_info",
            "interface" | "interface_list" => "network.routeros.interface_list",
            "ip.address" | "ip_addresses" => "network.routeros.ip_addresses",
            "firewall.filter" | "firewall_filter_list" => {
                "network.routeros.firewall_filter_list"
            }
            other => {
                return Err(DriverError::Unsupported(format!(
                    "read_state: unknown selector table `{other}`"
                )));
            }
        };
        let out = self
            .dispatch(&self.actor_id, capability, target, None, DEFAULT_TIMEOUT_SECS)
            .await
            .map_err(driver_err)?;
        Ok(StateDoc {
            target: target.to_string(),
            doc: serde_json::json!({
                "table": table,
                "command": out.command,
                "raw": out.stdout,
                "exit_status": out.exit_status,
            }),
        })
    }

    /// READ — read the filter table and surface it as findings. This driver does
    /// NOT decide the fix: every finding merely SUGGESTS `firewall_remove_rule`
    /// so the LLM/orchestrator can do root-cause and lift the suggestion into a
    /// plan step (FR-CON-20). The raw table is returned as one Info finding.
    async fn diagnose(&self, target: &str, symptom: &str) -> DriverResult<Vec<Finding>> {
        let out = self
            .dispatch(
                &self.actor_id,
                "network.routeros.firewall_filter_list",
                target,
                None,
                DEFAULT_TIMEOUT_SECS,
            )
            .await
            .map_err(driver_err)?;

        let finding = Finding {
            title: format!("firewall filter table for symptom `{symptom}`"),
            severity: Severity::Info,
            detail: out.stdout,
            // Surface the candidate remediation but leave the `match` param empty
            // — the LLM/orchestrator fills it from the offending row. We do NOT
            // pre-decide which rule to remove.
            suggested_capability: Some("network.routeros.firewall_remove_rule".into()),
            suggested_params: serde_json::json!({}),
        };
        Ok(vec![finding])
    }

    /// WRITE — apply a write capability with typed params. Subject to full guard
    /// policy + approval; the write disposition is derived server-side from the
    /// registered `Capability`. Returns a `Receipt` whose `changed` block echoes
    /// the params + emitted command so saga rollback can derive the inverse.
    async fn remediate(
        &self,
        target: &str,
        capability: &str,
        params: serde_json::Value,
    ) -> DriverResult<Receipt> {
        // Reject reads routed to the write verb — remediate is the write path.
        if !is_write_capability(capability) {
            return Err(DriverError::Unsupported(format!(
                "remediate: `{capability}` is not a RouterOS write capability"
            )));
        }
        let out = self
            .dispatch(
                &self.actor_id,
                capability,
                target,
                Some(&params),
                DEFAULT_TIMEOUT_SECS,
            )
            .await
            .map_err(driver_err)?;
        Ok(Receipt {
            capability: capability.to_string(),
            changed: serde_json::json!({
                "target": target,
                "params": params,
                "command": out.command,
                "exit_status": out.exit_status,
            }),
        })
    }
}

/// Map the crate-local error to the driver-layer error. `Param` rejections keep
/// their typed `ParamError` (so the injection-chokepoint contract surfaces as
/// `DriverError::Param`); everything else stringifies.
fn driver_err(e: RouterOsError) -> DriverError {
    match e {
        RouterOsError::Param(pe) => DriverError::Param(pe),
        RouterOsError::Broker(be) => DriverError::Broker(be.to_string()),
        RouterOsError::BadTarget(m) => DriverError::Other(format!("invalid target_ref: {m}")),
        RouterOsError::UnknownCapability(m) => DriverError::Unsupported(m),
        RouterOsError::MissingParam(m) => DriverError::Other(m),
        RouterOsError::WrongOpResult(m) => DriverError::Parse(m),
    }
}

fn is_write_capability(capability: &str) -> bool {
    matches!(
        capability,
        "network.routeros.firewall_add_drop_rule"
            | "network.routeros.firewall_remove_rule"
            | "network.routeros.user_ssh_key_import"
            | "network.routeros.user_ssh_key_remove"
    )
}

// -------------------------------------------------------------------------
// Agent bus adapter (FR-HAR-17)
// -------------------------------------------------------------------------

#[async_trait]
impl Agent for RouterOsDriver {
    fn id(&self) -> &AgentId {
        &self.id
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    #[instrument(skip(self, ctx), fields(agent = %self.id))]
    async fn handle(&self, env: AgentEnvelope, ctx: AgentContext) -> Result<(), CoreError> {
        // Each incoming envelope carries a JSON body shaped as `NetworkRequest`.
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

        // FR-HAR-17: carry the CALLER identity from the envelope's `from` field
        // as the audit/broker actor — NOT the driver's own agent id — so
        // audit/approval records the real requester.
        let actor_id = env.from.as_str();
        debug!(
            capability = %req.capability,
            target = %req.target_ref,
            actor = %actor_id,
            "dispatch"
        );

        let timeout_secs = req.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
        let result = self
            .dispatch(
                actor_id,
                &req.capability,
                &req.target_ref,
                req.params.as_ref(),
                timeout_secs,
            )
            .await;

        let resp = match result {
            Ok(out) => NetworkResponse::ok(out),
            Err(e) => {
                error!(error = %e, "routeros capability failed");
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

// -------------------------------------------------------------------------
// Capability catalog — copied verbatim from NetworkAgent::new (identical
// names / versions / compensating / irreversible dispositions).
// -------------------------------------------------------------------------

fn build_capabilities() -> Vec<Capability> {
    vec![
        Capability::read_only("network.routeros.system_info", Version::new(1, 0, 0)),
        Capability::read_only("network.routeros.interface_list", Version::new(1, 0, 0)),
        Capability::read_only("network.routeros.ip_addresses", Version::new(1, 0, 0)),
        Capability::read_only(
            "network.routeros.firewall_filter_list",
            Version::new(1, 0, 0),
        ),
        // Write capabilities. Both go through Guard's policy + approval-inbox
        // pre-flight in broker.execute (D19). The compensating pair is D18-saga
        // material: `firewall_add_drop_rule` is rolled back by
        // `firewall_remove_rule`, which is itself irreversible (a removed rule
        // is gone).
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
        // SSH user key rotation. RouterOS syntax:
        // `/user ssh-keys import file=<basename> user=<user>`. Compensating
        // capability is the removal of the new key.
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
            // A removed rule can't be auto-restored — operator confirmation
            // required on every invocation.
            irreversible: true,
        },
    ]
}

// -------------------------------------------------------------------------
// Command builder — moved from tool-network, `reject_shell_metachars`
// replaced by `param::validate` (the shared injection chokepoint).
// -------------------------------------------------------------------------

/// Build the RouterOS CLI command for a capability. Read capabilities take no
/// params and resolve to a fixed string; write capabilities interpolate from
/// `params` against the schema declared in the capability catalog.
///
/// # Param → `ParamClass` mapping (FR-CON-12)
///
/// | capability param       | ParamClass  | rationale                                              |
/// |------------------------|-------------|--------------------------------------------------------|
/// | `dst_address`          | `Cidr`      | an IP/`ip/prefix`; plan-directed. See NOTE below.      |
/// | `src_address`          | `Cidr`      | source IP/prefix.                                      |
/// | `in_interface`         | `Identifier`| interface name (`[A-Za-z0-9_-]`, no spaces).          |
/// | `comment`              | `SafeText`  | free text incl. space; shell metachars still rejected.|
/// | `firewall.match`       | `Identifier`| a `[find <expr>]` selector token; no spaces/metachars.|
/// | `user`                 | `Identifier`| RouterOS user name.                                   |
/// | `file`                 | `SafeText`  | a `.pub` basename — needs `.`, which `Identifier` bans.|
/// | `number`               | `Int`       | a row index from `ssh-keys print`.                    |
///
/// NOTE (decided beyond the plan): the catalog docstring says `dst_address` may
/// be a CIDR *or an address-list name*. The plan directs `dst_address -> Cidr`,
/// so an address-list *name* (e.g. `tiktok-domains`) is only accepted when it is
/// hex-digit/`.`/`:`/`/`-shaped — a plain alphabetic name would be rejected by
/// `Cidr`. We follow the plan's mapping (Cidr) as the safer choice; a future
/// commit can widen this to a union class if address-list-by-name is needed.
///
/// Failures: unknown capability, missing required params, or params that fail
/// their declared `ParamClass` (`param::validate` — the chokepoint; no `Op` is
/// built on violation).
fn build_command(
    capability: &str,
    params: Option<&serde_json::Value>,
) -> Result<String, RouterOsError> {
    match capability {
        "network.routeros.system_info" => Ok("/system identity print".into()),
        "network.routeros.interface_list" => Ok("/interface print".into()),
        "network.routeros.ip_addresses" => Ok("/ip address print".into()),
        "network.routeros.firewall_filter_list" => Ok("/ip firewall filter print".into()),
        "network.routeros.firewall_add_drop_rule" => {
            let p = params.ok_or_else(|| {
                RouterOsError::MissingParam("firewall_add_drop_rule requires params".into())
            })?;
            let dst = p
                .get("dst_address")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    RouterOsError::MissingParam(
                        "firewall_add_drop_rule.dst_address is required".into(),
                    )
                })?;
            param::validate(dst, &ParamClass::Cidr)?;
            let mut cmd =
                format!("/ip firewall filter add chain=forward action=drop dst-address={dst}");
            if let Some(src) = p.get("src_address").and_then(|v| v.as_str()) {
                param::validate(src, &ParamClass::Cidr)?;
                cmd.push_str(&format!(" src-address={src}"));
            }
            if let Some(iface) = p.get("in_interface").and_then(|v| v.as_str()) {
                param::validate(iface, &ParamClass::Identifier)?;
                cmd.push_str(&format!(" in-interface={iface}"));
            }
            if let Some(comment) = p.get("comment").and_then(|v| v.as_str()) {
                param::validate(comment, &ParamClass::SafeText)?;
                cmd.push_str(&format!(" comment=\"{comment}\""));
            }
            Ok(cmd)
        }
        "network.routeros.firewall_remove_rule" => {
            let p = params.ok_or_else(|| {
                RouterOsError::MissingParam("firewall_remove_rule requires params".into())
            })?;
            let m = p.get("match").and_then(|v| v.as_str()).ok_or_else(|| {
                RouterOsError::MissingParam("firewall_remove_rule.match is required".into())
            })?;
            param::validate(m, &ParamClass::Identifier)?;
            Ok(format!("/ip firewall filter remove [find {m}]"))
        }
        "network.routeros.user_ssh_key_import" => {
            let p = params.ok_or_else(|| {
                RouterOsError::MissingParam("user_ssh_key_import requires params".into())
            })?;
            let user = p.get("user").and_then(|v| v.as_str()).ok_or_else(|| {
                RouterOsError::MissingParam("user_ssh_key_import.user is required".into())
            })?;
            let file = p.get("file").and_then(|v| v.as_str()).ok_or_else(|| {
                RouterOsError::MissingParam("user_ssh_key_import.file is required".into())
            })?;
            param::validate(user, &ParamClass::Identifier)?;
            param::validate(file, &ParamClass::SafeText)?;
            Ok(format!("/user ssh-keys import user={user} public-key-file={file}"))
        }
        "network.routeros.user_ssh_key_remove" => {
            let p = params.ok_or_else(|| {
                RouterOsError::MissingParam("user_ssh_key_remove requires params".into())
            })?;
            let n = p.get("number").and_then(|v| v.as_str()).ok_or_else(|| {
                RouterOsError::MissingParam("user_ssh_key_remove.number is required".into())
            })?;
            param::validate(n, &ParamClass::Int)?;
            Ok(format!("/user ssh-keys remove {n}"))
        }
        other => Err(RouterOsError::UnknownCapability(other.to_string())),
    }
}

// -------------------------------------------------------------------------
// Wire types — moved from tool-network (this crate now owns them).
// -------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRequest {
    pub capability: String,
    pub target_ref: String,
    #[serde(default)]
    pub timeout_secs: Option<u32>,
    /// Write-capability params (validated by `build_command`). `None` for read
    /// capabilities.
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

/// Crate-local error. `Param` carries the typed `param::ParamError` so the
/// injection-chokepoint rejection is distinguishable from a broker/target error.
#[derive(Debug, Error)]
pub enum RouterOsError {
    #[error("unknown capability: {0}")]
    UnknownCapability(String),
    #[error("missing param: {0}")]
    MissingParam(String),
    #[error("parameter validation: {0}")]
    Param(#[from] param::ParamError),
    #[error("invalid target_ref: {0}")]
    BadTarget(String),
    #[error("broker: {0}")]
    Broker(BrokerError),
    #[error("unexpected OpResult variant: {0}")]
    WrongOpResult(String),
}

// Suppress unused warning on Recipient — re-exported so callers can build
// envelopes addressed to this driver by capability.
#[allow(dead_code)]
fn _hint(_: Recipient) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_caps_build_fixed_commands() {
        assert_eq!(
            build_command("network.routeros.system_info", None).unwrap(),
            "/system identity print"
        );
        assert_eq!(
            build_command("network.routeros.interface_list", None).unwrap(),
            "/interface print"
        );
        assert_eq!(
            build_command("network.routeros.firewall_filter_list", None).unwrap(),
            "/ip firewall filter print"
        );
    }

    #[test]
    fn tiktok_block_command_generation() {
        // The Phase-8 locked vertical, now at the build_command layer with
        // param::validate. `dst_address` is a CIDR here (the plan maps it to
        // Cidr); `in_interface` is an Identifier; `comment` is SafeText.
        let cmd = build_command(
            "network.routeros.firewall_add_drop_rule",
            Some(&serde_json::json!({
                "dst_address": "185.60.216.0/24",
                "in_interface": "vlan20",
                "comment": "block tiktok phase8 demo"
            })),
        )
        .unwrap();
        assert!(cmd.starts_with("/ip firewall filter add chain=forward action=drop"));
        assert!(cmd.contains("dst-address=185.60.216.0/24"));
        assert!(cmd.contains("in-interface=vlan20"));
        assert!(cmd.contains("comment=\"block tiktok phase8 demo\""));
    }

    #[test]
    fn metachar_rejection_via_param_validate() {
        // The historical reject_shell_metachars contract, now routed through
        // param::validate. A `;` in a Cidr param must be rejected.
        let err = build_command(
            "network.routeros.firewall_add_drop_rule",
            Some(&serde_json::json!({ "dst_address": "10.0.0.1; /system shutdown" })),
        )
        .unwrap_err();
        assert!(
            format!("{err}").contains("disallowed char"),
            "expected disallowed-char rejection, got: {err}"
        );
    }

    #[test]
    fn comment_rejects_shell_metachars() {
        // SafeText rejects every shell metachar even though it allows spaces.
        for meta in [';', '|', '&', '$', '`', '<', '>'] {
            let err = build_command(
                "network.routeros.firewall_add_drop_rule",
                Some(&serde_json::json!({
                    "dst_address": "10.0.0.0/24",
                    "comment": format!("ok{meta}bad")
                })),
            )
            .unwrap_err();
            assert!(
                format!("{err}").contains("disallowed char"),
                "meta `{meta}` must be rejected in comment"
            );
        }
    }

    #[test]
    fn ssh_key_import_command_generation() {
        let cmd = build_command(
            "network.routeros.user_ssh_key_import",
            Some(&serde_json::json!({ "user": "admin", "file": "newkey.pub" })),
        )
        .unwrap();
        assert_eq!(cmd, "/user ssh-keys import user=admin public-key-file=newkey.pub");
    }

    #[test]
    fn ssh_key_remove_requires_int() {
        // `number` maps to ParamClass::Int — a non-numeric value is rejected.
        assert!(build_command(
            "network.routeros.user_ssh_key_remove",
            Some(&serde_json::json!({ "number": "3" })),
        )
        .is_ok());
        let err = build_command(
            "network.routeros.user_ssh_key_remove",
            Some(&serde_json::json!({ "number": "3; reboot" })),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("not a valid Int"));
    }

    #[test]
    fn firewall_remove_match_is_identifier() {
        // `match` maps to Identifier — a bare token passes, spaces/metachars fail.
        assert!(build_command(
            "network.routeros.firewall_remove_rule",
            Some(&serde_json::json!({ "match": "3" })),
        )
        .is_ok());
        let err = build_command(
            "network.routeros.firewall_remove_rule",
            Some(&serde_json::json!({ "match": "3; reboot" })),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("disallowed char"));
    }

    #[test]
    fn unknown_capability_rejected() {
        let err = build_command("network.routeros.nope", None).unwrap_err();
        assert!(format!("{err}").contains("unknown capability"));
    }

    #[test]
    fn exec_request_carries_caller_actor_and_capability_meta() {
        // FR-HAR-17: the actor the bus adapter sources from `env.from` must land
        // verbatim on ExecRequest.actor_id — NOT a static driver id. Here we feed
        // a distinct caller identity and assert it propagates, and that the
        // write cap's meta drives the server-side write disposition.
        let caps = build_capabilities();
        let req = build_exec_request(
            "user:alice", // stand-in for env.from
            "network.routeros.firewall_add_drop_rule",
            "target://mikrotik-edge",
            "/ip firewall filter add chain=forward action=drop dst-address=10.0.0.0/24",
            15,
            &caps,
        )
        .unwrap();
        assert_eq!(req.actor_id, "user:alice", "env.from must become the actor");
        // Write cap → capability_meta attached, read-only derived false.
        assert!(req.capability_meta.is_some());
        assert!(!req.is_read_only, "write cap must not be flagged read-only");
        assert_eq!(
            req.capability.as_deref(),
            Some("network.routeros.firewall_add_drop_rule")
        );
    }

    #[test]
    fn exec_request_flags_read_only_for_read_caps() {
        let caps = build_capabilities();
        let req = build_exec_request(
            "user:bob",
            "network.routeros.interface_list",
            "target://mikrotik-edge",
            "/interface print",
            10,
            &caps,
        )
        .unwrap();
        assert_eq!(req.actor_id, "user:bob");
        assert!(req.is_read_only, "read cap must derive read-only server-side");
    }

    #[test]
    fn catalog_has_expected_dispositions() {
        let caps = build_capabilities();
        let get = |n: &str| caps.iter().find(|c| c.name == n).unwrap();
        assert!(get("network.routeros.system_info").is_read());
        assert!(!get("network.routeros.firewall_add_drop_rule").is_read());
        assert!(get("network.routeros.firewall_remove_rule").irreversible);
        assert!(get("network.routeros.user_ssh_key_remove").irreversible);
        assert_eq!(
            get("network.routeros.firewall_add_drop_rule")
                .compensating
                .as_ref()
                .unwrap()
                .name,
            "network.routeros.firewall_remove_rule"
        );
    }
}
