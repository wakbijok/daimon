//! The generic, declarative `ConnectorDriver` (P2 commit 9, SDS §5.4).
//!
//! Where [`crate::Driver`] implementers like the RouterOS reference driver hard-
//! code their command builders in Rust, `ConnectorDriver` interprets *data*: a
//! directory of `.toml` connector profiles. Each profile describes ONE target
//! class (Compute / Network / Storage / Orchestrator / Firewall) reached over
//! ONE transport (currently `rest`), and lists `[[capability]]` blocks — a name,
//! version, param table, and an `op` template. At construction the driver:
//!
//! 1. loads every profile in the directory,
//! 2. projects each `[[capability]]` into a [`daimon_core::Capability`] (so they
//!    register via [`Agent::capabilities`] when the supervisor spawns the driver
//!    and appear in the chat/planner catalogs), and
//! 3. remembers the `op` template + per-param [`ParamClass`] + parser for
//!    dispatch time.
//!
//! # This is the SECOND driver
//!
//! Adding it makes by-capability routing meaningful: `orchestrator.k8s.*` resolves to
//! this driver over REST while `network.routeros.*` resolves to the RouterOS
//! driver over SSH — the harness routes by capability, blind to transport
//! (AC-P2-10).
//!
//! # Security is inherited, not re-implemented (D21 + FR-CON-12)
//!
//! Exactly like the code drivers, `ConnectorDriver` reaches infrastructure ONLY
//! by building an `ExecRequest` and submitting it to `Broker::execute`. It never
//! touches vault/transport/inventory — the target's REST host + credential come
//! from inventory (the broker resolves them by `target_ref`); the profile
//! supplies only the path template + method. A declarative profile therefore
//! CANNOT bypass the guard/vault/audit path, and it CANNOT bypass the injection
//! chokepoint: every param is run through [`crate::param::validate`] against its
//! declared class BEFORE it is substituted into the `Op::Http` template. A
//! profile that omits a class, or supplies a value that violates one, is
//! rejected and no `Op` is built.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use daimon_broker::{
    AuthScheme, Broker, ExecRequest, HttpMethod, Op, OpResult, TargetRef as InvTargetRef,
};
use daimon_core::{
    Agent, AgentContext, AgentEnvelope, AgentId, Capability, CompensatingCapability, CoreError,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, instrument, warn};

use crate::param::{self, ParamClass};
use crate::{DriverError, TargetClass};

const DEFAULT_TIMEOUT_SECS: u32 = 30;

// =========================================================================
// Profile schema (the on-disk `.toml` contract)
// =========================================================================

/// A parsed connector profile — one `.toml` file, one target class + transport.
#[derive(Debug, Clone, Deserialize)]
pub struct ConnectorProfile {
    /// Human-readable profile name (e.g. `"k8s"`). Informational.
    pub name: String,
    /// The semantic target class this profile serves.
    pub class: ProfileClass,
    /// The transport kind. Currently only `rest` is interpreted.
    pub transport: ProfileTransport,
    /// How the REST transport authenticates to this target. Omitted = Bearer
    /// (back-compat). A non-Bearer API (Proxmox `PVEAPIToken`, etc.) sets an
    /// `[auth]` block. The secret token still comes from the vault; this only
    /// names the header + value format.
    #[serde(default)]
    pub auth: Option<ProfileAuth>,
    /// The capabilities this profile declares.
    #[serde(default, rename = "capability")]
    pub capabilities: Vec<ProfileCapability>,
}

/// The `[auth]` sub-table of a connector profile.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileAuth {
    /// `"bearer"` (default) | `"header"` | `"none"`.
    #[serde(default)]
    pub scheme: Option<String>,
    /// For `scheme = "header"`: the header name, e.g. `"Authorization"`.
    #[serde(default)]
    pub header: Option<String>,
    /// For `scheme = "header"`: the value template with a `{token}` slot, e.g.
    /// `"PVEAPIToken {token}"`.
    #[serde(default)]
    pub value: Option<String>,
}

impl ProfileAuth {
    /// Resolve to the transport `AuthScheme`. A `header` scheme needs both
    /// `header` + `value`; anything malformed falls back to `Bearer`.
    fn to_scheme(&self) -> AuthScheme {
        match self.scheme.as_deref() {
            Some("none") => AuthScheme::None,
            Some("header") => match (&self.header, &self.value) {
                (Some(h), Some(v)) => AuthScheme::Header {
                    header: h.clone(),
                    value: v.clone(),
                },
                _ => AuthScheme::Bearer,
            },
            _ => AuthScheme::Bearer,
        }
    }
}

/// Profile `class` — mirrors [`TargetClass`] but is a profile-schema type so the
/// TOML surface is decoupled from the internal enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileClass {
    Compute,
    Network,
    Storage,
    Orchestrator,
    Firewall,
}

impl From<ProfileClass> for TargetClass {
    fn from(c: ProfileClass) -> Self {
        match c {
            ProfileClass::Compute => TargetClass::Compute,
            ProfileClass::Network => TargetClass::Network,
            ProfileClass::Storage => TargetClass::Storage,
            ProfileClass::Orchestrator => TargetClass::Orchestrator,
            ProfileClass::Firewall => TargetClass::Firewall,
        }
    }
}

/// Profile `transport`. Only `rest` is interpreted in P2; the slot exists so a
/// future SSH/SNMP declarative profile can be added without a schema break.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileTransport {
    Rest,
    /// SSH — the driver renders `Op::ShellCommand` from a `command` template.
    Ssh,
}

/// One `[[capability]]` block.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileCapability {
    pub name: String,
    /// Semver of the capability contract, e.g. `"1.0.0"`.
    pub version: Version,
    #[serde(default)]
    pub description: Option<String>,
    /// The compensating capability name for saga rollback (a write's inverse).
    #[serde(default)]
    pub compensating: Option<String>,
    /// Marks an irreversible write (no auto-rollback).
    #[serde(default)]
    pub irreversible: bool,
    /// Explicit read-only override. Normally omitted — dispatch derives the
    /// read/write disposition server-side from the projected `Capability`. When
    /// `true`, the projected capability is built via `Capability::read_only`
    /// (used for the `.status` read cap whose name already carries a read verb).
    #[serde(default)]
    pub read_only: bool,
    /// param-name → param-class map. Every `{param}` slot in the op template
    /// MUST have an entry here — that is the injection chokepoint contract.
    #[serde(default)]
    pub params: BTreeMap<String, ProfileParamClass>,
    /// The transport operation template.
    pub op: ProfileOp,
    /// Optional parse hint applied to the `OpResult` before it becomes the
    /// reply body. `json` wraps the response document; absent = pass-through.
    #[serde(default)]
    pub parse: Option<ParseHint>,
}

/// The declared character class for a param, parsed from a compact string form
/// so profiles stay terse: `"safe_text" | "cidr" | "identifier" | "int" |
/// "enum:a,b,c"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileParamClass(pub ParamClass);

impl<'de> Deserialize<'de> for ProfileParamClass {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let class = match s.as_str() {
            "safe_text" => ParamClass::SafeText,
            "cidr" => ParamClass::Cidr,
            "identifier" => ParamClass::Identifier,
            "int" => ParamClass::Int,
            other => {
                if let Some(members) = other.strip_prefix("enum:") {
                    let variants: Vec<String> = members
                        .split(',')
                        .map(|m| m.trim().to_owned())
                        .filter(|m| !m.is_empty())
                        .collect();
                    if variants.is_empty() {
                        return Err(serde::de::Error::custom(
                            "enum: param class needs at least one member",
                        ));
                    }
                    ParamClass::Enum(variants)
                } else {
                    return Err(serde::de::Error::custom(format!(
                        "unknown param class `{other}` (expected safe_text|cidr|identifier|int|enum:a,b,c)"
                    )));
                }
            }
        };
        Ok(ProfileParamClass(class))
    }
}

/// The `op` sub-table. For REST: `method` + `path` template (+ optional `body`
/// template). For SSH: a `command` template. `{param}` slots are substituted from
/// validated params (the injection chokepoint — never raw operator/LLM text).
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileOp {
    /// REST method. Defaults to GET; ignored for SSH ops.
    #[serde(default)]
    pub method: ProfileHttpMethod,
    /// REST path template with `{param}` slots, e.g.
    /// `/api2/json/nodes/{node}/qemu/{vmid}/status/current`. Empty for SSH ops.
    #[serde(default)]
    pub path: String,
    /// Optional JSON body template (REST). Any string leaf containing `{param}`
    /// is substituted; other leaves pass through verbatim.
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    /// SSH command template with `{param}` slots, e.g.
    /// `systemctl restart {service}`. Present for `transport = "ssh"` ops.
    #[serde(default)]
    pub command: Option<String>,
    /// Optional SSH command timeout (seconds); defaults to 30.
    #[serde(default)]
    pub timeout_secs: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ProfileHttpMethod {
    #[default]
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl From<ProfileHttpMethod> for HttpMethod {
    fn from(m: ProfileHttpMethod) -> Self {
        match m {
            ProfileHttpMethod::Get => HttpMethod::Get,
            ProfileHttpMethod::Post => HttpMethod::Post,
            ProfileHttpMethod::Put => HttpMethod::Put,
            ProfileHttpMethod::Patch => HttpMethod::Patch,
            ProfileHttpMethod::Delete => HttpMethod::Delete,
        }
    }
}

/// Parse hint for the `OpResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseHint {
    /// Wrap the response document (already JSON from `OpResult::Http.body`)
    /// straight through as the result payload.
    Json,
}

// =========================================================================
// Errors
// =========================================================================

/// Errors from loading profiles or dispatching a declared capability.
#[derive(Debug, thiserror::Error)]
pub enum ConnectorError {
    #[error("io reading connector dir `{path}`: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("parse profile `{path}`: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
    #[error("unknown capability `{0}`")]
    UnknownCapability(String),
    #[error("missing required param `{param}` for capability `{capability}`")]
    MissingParam { capability: String, param: String },
    #[error("param `{param}` has no declared class in capability `{capability}` — a `{{param}}` slot cannot be substituted without a class (injection chokepoint)")]
    UndeclaredParamClass { capability: String, param: String },
    #[error("parameter validation for `{param}`: {source}")]
    Param {
        param: String,
        source: param::ParamError,
    },
    #[error("invalid target_ref `{0}`")]
    BadTarget(String),
    #[error("broker: {0}")]
    Broker(String),
    #[error("unexpected OpResult variant: {0}")]
    WrongOpResult(String),
    #[error("malformed connector profile: {0}")]
    BadProfile(String),
}

impl From<ConnectorError> for DriverError {
    fn from(e: ConnectorError) -> Self {
        match e {
            ConnectorError::Param { source, .. } => DriverError::Param(source),
            ConnectorError::Broker(m) => DriverError::Broker(m),
            ConnectorError::UnknownCapability(m) => DriverError::Unsupported(m),
            ConnectorError::WrongOpResult(m) => DriverError::Parse(m),
            other => DriverError::Other(other.to_string()),
        }
    }
}

// =========================================================================
// The driver
// =========================================================================

/// Generic connector driver: interprets declarative `.toml` profiles.
///
/// Holds an `Arc<Broker>` for credential-safe dispatch (SAME path as the code
/// drivers) and the set of loaded profiles. One `ConnectorDriver` can serve
/// several profiles, but all profiles it holds must share the driver's `class`
/// (the harness resolves a driver per class). In P2 we load a single class per
/// driver instance; the loader picks the class of the FIRST profile and skips
/// any profile of a different class with a warning.
pub struct ConnectorDriver {
    id: AgentId,
    broker: Arc<Broker>,
    class: TargetClass,
    /// Projected capabilities (registered via `Agent::capabilities`).
    capabilities: Vec<Capability>,
    /// The declared capabilities, indexed by name, for dispatch (each carries
    /// its source profile's auth scheme + transport).
    declared: BTreeMap<String, DeclaredCap>,
    /// Fallback audit identity for the synchronous entrypoints. The bus adapter
    /// always overrides this with `env.from` (FR-HAR-17).
    actor_id: String,
}

/// A declared capability plus the dispatch context from its source profile: how
/// to authenticate (REST) and which transport op to render.
struct DeclaredCap {
    cap: ProfileCapability,
    auth: AuthScheme,
    transport: ProfileTransport,
}

impl ConnectorDriver {
    /// Load every `.toml` profile from `dir`, grouped by target CLASS — one
    /// `ConnectorDriver` per class (a driver instance is single-class by design).
    /// A site drops the connectors its equipment needs (k8s→orchestrator,
    /// redfish→compute, a firewall, …) and each class gets its own driver with a
    /// distinct agent id (`{id_prefix}:{class}`), so heterogeneous targets
    /// coexist rather than one class silently shadowing the rest. Returns an
    /// empty Vec if the dir is absent or has no profiles (caller skips).
    pub fn from_dir(
        id_prefix: &str,
        broker: Arc<Broker>,
        dir: &Path,
        actor_id: &str,
    ) -> Result<Vec<Self>, ConnectorError> {
        if !dir.exists() {
            debug!(dir = %dir.display(), "connector dir absent — skipping");
            return Ok(Vec::new());
        }
        let mut profiles = Vec::new();
        let entries = std::fs::read_dir(dir).map_err(|e| ConnectorError::Io {
            path: dir.display().to_string(),
            source: e,
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| ConnectorError::Io {
                path: dir.display().to_string(),
                source: e,
            })?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }
            let text = std::fs::read_to_string(&path).map_err(|e| ConnectorError::Io {
                path: path.display().to_string(),
                source: e,
            })?;
            let profile: ConnectorProfile =
                toml::from_str(&text).map_err(|e| ConnectorError::Parse {
                    path: path.display().to_string(),
                    source: e,
                })?;
            debug!(profile = %profile.name, caps = profile.capabilities.len(), "loaded connector profile");
            profiles.push(profile);
        }
        if profiles.is_empty() {
            debug!(dir = %dir.display(), "connector dir empty — skipping");
            return Ok(Vec::new());
        }
        // Group profiles by class — one driver per class.
        let mut groups: Vec<(TargetClass, Vec<ConnectorProfile>)> = Vec::new();
        for p in profiles {
            let c: TargetClass = p.class.into();
            if let Some(g) = groups.iter_mut().find(|(gc, _)| *gc == c) {
                g.1.push(p);
            } else {
                groups.push((c, vec![p]));
            }
        }
        let drivers = groups
            .into_iter()
            .map(|(class, profs)| {
                let id = AgentId::new(format!("{id_prefix}:{}", class_slug(class)));
                Self::from_profiles(id, broker.clone(), profs, actor_id.to_string())
            })
            .collect();
        Ok(drivers)
    }

    /// Build from already-parsed profiles. The driver's class is taken from the
    /// first profile; profiles of a differing class are skipped with a warning.
    pub fn from_profiles(
        id: AgentId,
        broker: Arc<Broker>,
        profiles: Vec<ConnectorProfile>,
        actor_id: impl Into<String>,
    ) -> Self {
        let class: TargetClass = profiles
            .first()
            .map(|p| p.class.into())
            .unwrap_or(TargetClass::Compute);

        let mut capabilities = Vec::new();
        let mut declared = BTreeMap::new();
        for profile in profiles {
            let profile_class: TargetClass = profile.class.into();
            if profile_class != class {
                warn!(
                    profile = %profile.name,
                    "connector profile class differs from driver class — skipping (one class per driver instance)"
                );
                continue;
            }
            // Resolve the profile's auth scheme once and denormalise it onto each
            // declared cap, so dispatch knows how to authenticate without
            // re-finding the source profile.
            let auth = profile
                .auth
                .as_ref()
                .map(|a| a.to_scheme())
                .unwrap_or_default();
            let transport = profile.transport;
            for cap in profile.capabilities {
                capabilities.push(project_capability(&cap));
                declared.insert(
                    cap.name.clone(),
                    DeclaredCap {
                        cap,
                        auth: auth.clone(),
                        transport,
                    },
                );
            }
        }

        Self {
            id,
            broker,
            class,
            capabilities,
            declared,
            actor_id: actor_id.into(),
        }
    }

    /// The projected capabilities (also the `Agent::capabilities` set).
    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    /// The core dispatch: resolve the declared capability, VALIDATE every param
    /// (injection chokepoint), render the `Op::Http`, attach the projected
    /// `Capability`, and submit through `Broker::execute` — the SAME security
    /// path as the code drivers. `actor_id` is the audit identity (from
    /// `env.from` on the bus path).
    async fn dispatch(
        &self,
        actor_id: &str,
        capability: &str,
        target_ref: &str,
        params: &serde_json::Value,
        timeout_secs: u32,
    ) -> Result<ConnectorOutput, ConnectorError> {
        let d = self
            .declared
            .get(capability)
            .ok_or_else(|| ConnectorError::UnknownCapability(capability.to_string()))?;
        let decl = &d.cap;

        // Validate + collect every param BEFORE rendering the Op. This is the
        // injection chokepoint: a slot cannot be filled by a value that does not
        // satisfy its declared ParamClass, and a slot without a declared class
        // is rejected outright. (Load-bearing for SSH — a command template's
        // `{param}` can never be raw operator/LLM text.)
        let mut validated: BTreeMap<String, String> = BTreeMap::new();
        for (pname, pclass) in &decl.params {
            let raw = extract_param(params, pname).ok_or_else(|| ConnectorError::MissingParam {
                capability: capability.to_string(),
                param: pname.clone(),
            })?;
            param::validate(&raw, &pclass.0).map_err(|e| ConnectorError::Param {
                param: pname.clone(),
                source: e,
            })?;
            validated.insert(pname.clone(), raw);
        }

        // Render the transport Op from the template using ONLY validated params.
        let op = match d.transport {
            ProfileTransport::Rest => render_http_op(&decl.op, capability, &validated, &d.auth)?,
            ProfileTransport::Ssh => render_ssh_op(&decl.op, capability, &validated, timeout_secs)?,
        };

        // Build + submit the ExecRequest through the broker (guard/vault/audit).
        let target =
            InvTargetRef::parse(target_ref).map_err(|e| ConnectorError::BadTarget(format!("{e}")))?;
        let cap_meta = project_capability(decl);
        let exec_req =
            ExecRequest::new(actor_id.to_string(), target, op).with_capability_meta(cap_meta);

        let op_result = self
            .broker
            .execute(exec_req)
            .await
            .map_err(|e| ConnectorError::Broker(e.to_string()))?;

        // Run the parser over the OpResult, producing the reply payload.
        let (status, payload) = parse_result(decl.parse, op_result)?;
        Ok(ConnectorOutput {
            capability: capability.to_string(),
            status,
            payload,
        })
    }
}

/// Project a `[[capability]]` block into a `daimon_core::Capability`. This is
/// what registers via `Agent::capabilities` and drives the server-side read-only
/// A stable, lowercase slug for a target class — used to give each per-class
/// `ConnectorDriver` a distinct agent id (`agent:connector:<slug>`).
fn class_slug(c: TargetClass) -> &'static str {
    match c {
        TargetClass::Compute => "compute",
        TargetClass::Network => "network",
        TargetClass::Storage => "storage",
        TargetClass::Orchestrator => "orchestrator",
        TargetClass::Firewall => "firewall",
    }
}

/// derivation. When `read_only = true` the capability is built via
/// `read_only()`; otherwise the compensating/irreversible dispositions carry
/// through and the name-verb heuristic decides `is_read()`.
fn project_capability(cap: &ProfileCapability) -> Capability {
    if cap.read_only {
        return Capability::read_only(cap.name.clone(), cap.version.clone());
    }
    Capability {
        name: cap.name.clone(),
        version: cap.version.clone(),
        description: cap.description.clone(),
        schema: Some(schema_from_params(&cap.params)),
        compensating: cap.compensating.clone().map(|name| CompensatingCapability {
            name,
            version_req: None,
        }),
        irreversible: cap.irreversible,
    }
}

/// Build a minimal JSON-Schema object from the profile param table so the chat
/// tool catalog + planner have a params contract to render.
fn schema_from_params(params: &BTreeMap<String, ProfileParamClass>) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for (name, class) in params {
        let ty = match &class.0 {
            ParamClass::Int => "integer",
            _ => "string",
        };
        properties.insert(name.clone(), serde_json::json!({ "type": ty }));
        required.push(name.clone());
    }
    serde_json::json!({
        "type": "object",
        "required": required,
        "properties": properties,
    })
}

/// Pull a param value out of the request `params` object as a string. Numbers
/// are stringified (so an `int`-class param can arrive as a JSON number or
/// string). Returns `None` if absent or a non-scalar.
fn extract_param(params: &serde_json::Value, name: &str) -> Option<String> {
    match params.get(name)? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Substitute `{param}` slots in the path (and any string leaf of the body
/// template) with validated param values, producing an `Op::Http`.
fn render_http_op(
    op: &ProfileOp,
    capability: &str,
    validated: &BTreeMap<String, String>,
    auth: &AuthScheme,
) -> Result<Op, ConnectorError> {
    let path = substitute(&op.path, capability, validated)?;
    let body = match &op.body {
        Some(v) => Some(substitute_json(v, capability, validated)?),
        None => None,
    };
    Ok(Op::Http {
        method: op.op_method(),
        path,
        headers: BTreeMap::new(),
        body,
        auth: auth.clone(),
    })
}

/// Render an `Op::ShellCommand` from an SSH capability's `command` template.
/// Every `{param}` was validated at the injection chokepoint before this — the
/// template itself is fixed in the profile, never operator/LLM text — so no
/// shell metacharacter can be injected through a substituted value.
fn render_ssh_op(
    op: &ProfileOp,
    capability: &str,
    validated: &BTreeMap<String, String>,
    timeout_secs: u32,
) -> Result<Op, ConnectorError> {
    let template = op.command.as_ref().ok_or_else(|| {
        ConnectorError::BadProfile(format!(
            "ssh capability `{capability}` has no `command` template"
        ))
    })?;
    let command = substitute(template, capability, validated)?;
    Ok(Op::ShellCommand {
        command,
        timeout_secs: op.timeout_secs.unwrap_or(timeout_secs),
    })
}

impl ProfileOp {
    fn op_method(&self) -> HttpMethod {
        self.method.into()
    }
}

/// Replace every `{name}` in `template` with `validated[name]`. An unresolved
/// slot (no validated value) is a missing-param error — a template can never
/// emit an un-substituted `{slot}` into a live URL.
fn substitute(
    template: &str,
    capability: &str,
    validated: &BTreeMap<String, String>,
) -> Result<String, ConnectorError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or_else(|| ConnectorError::MissingParam {
            capability: capability.to_string(),
            param: "<unterminated slot>".to_string(),
        })?;
        let name = &after[..close];
        let value = validated
            .get(name)
            .ok_or_else(|| ConnectorError::MissingParam {
                capability: capability.to_string(),
                param: name.to_string(),
            })?;
        out.push_str(value);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Recursively substitute `{param}` slots in every string leaf of a JSON body
/// template.
fn substitute_json(
    value: &serde_json::Value,
    capability: &str,
    validated: &BTreeMap<String, String>,
) -> Result<serde_json::Value, ConnectorError> {
    match value {
        serde_json::Value::String(s) => {
            Ok(serde_json::Value::String(substitute(s, capability, validated)?))
        }
        serde_json::Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(substitute_json(item, capability, validated)?);
            }
            Ok(serde_json::Value::Array(out))
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), substitute_json(v, capability, validated)?);
            }
            Ok(serde_json::Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

/// Turn the transport's `OpResult` into `(status, payload)`. REST returns
/// `OpResult::Http`; the `json` parse hint (or the default) passes the response
/// body straight through as a JSON document.
fn parse_result(
    _hint: Option<ParseHint>,
    result: OpResult,
) -> Result<(i32, serde_json::Value), ConnectorError> {
    match result {
        OpResult::Http { status, body, .. } => Ok((status as i32, body)),
        OpResult::Structured { doc } => Ok((0, doc)),
        OpResult::ShellCommand {
            stdout,
            stderr,
            exit_status,
        } => Ok((
            exit_status,
            serde_json::json!({
                "stdout": stdout,
                "stderr": stderr,
                "exit_status": exit_status,
            }),
        )),
        other => Err(ConnectorError::WrongOpResult(format!("{other:?}"))),
    }
}

// =========================================================================
// Driver trait impl
// =========================================================================

#[async_trait]
impl crate::Driver for ConnectorDriver {
    fn class(&self) -> TargetClass {
        self.class
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    async fn describe(&self, target: &str) -> crate::DriverResult<crate::TargetShape> {
        Ok(crate::TargetShape {
            class: self.class,
            identity: serde_json::json!({ "target": target }),
            summary: format!("{:?} connector target {target}", self.class),
        })
    }

    async fn read_state(
        &self,
        target: &str,
        selector: serde_json::Value,
    ) -> crate::DriverResult<crate::StateDoc> {
        // The selector names the read capability + carries its params:
        // {"capability": "orchestrator.k8s.pod.status", "params": {...}}.
        let capability = selector
            .get("capability")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                DriverError::Unsupported(
                    "read_state selector must carry a `capability` field".into(),
                )
            })?;
        let params = selector.get("params").cloned().unwrap_or(serde_json::json!({}));
        let out = self
            .dispatch(&self.actor_id, capability, target, &params, DEFAULT_TIMEOUT_SECS)
            .await?;
        Ok(crate::StateDoc {
            target: target.to_string(),
            doc: serde_json::json!({
                "capability": out.capability,
                "status": out.status,
                "result": out.payload,
            }),
        })
    }

    async fn diagnose(
        &self,
        _target: &str,
        symptom: &str,
    ) -> crate::DriverResult<Vec<crate::Finding>> {
        // The declarative connector does not encode diagnostic heuristics — it
        // surfaces the symptom for the LLM to reason over with its read caps.
        Ok(vec![crate::Finding {
            title: format!("connector `{}` has no built-in diagnostics", self.id),
            severity: crate::Severity::Info,
            detail: format!("symptom `{symptom}`: use the read capabilities to gather state"),
            suggested_capability: None,
            suggested_params: serde_json::json!({}),
        }])
    }

    async fn remediate(
        &self,
        target: &str,
        capability: &str,
        params: serde_json::Value,
    ) -> crate::DriverResult<crate::Receipt> {
        let out = self
            .dispatch(&self.actor_id, capability, target, &params, DEFAULT_TIMEOUT_SECS)
            .await?;
        Ok(crate::Receipt {
            capability: capability.to_string(),
            changed: serde_json::json!({
                "target": target,
                "params": params,
                "status": out.status,
                "result": out.payload,
            }),
        })
    }
}

// =========================================================================
// Agent bus adapter (FR-HAR-17) — decodes the SAME request body the harness
// sends and replies with a NetworkResponse-compatible body so chat/orchestrator
// decode replies uniformly across drivers.
// =========================================================================

/// The request body the harness sends over the bus — identical shape to the
/// RouterOS driver's `NetworkRequest` (decoded here with serde_json directly, so
/// this crate does NOT depend on the routeros crate).
#[derive(Debug, Clone, Deserialize)]
struct ConnectorRequest {
    capability: String,
    target_ref: String,
    #[serde(default)]
    timeout_secs: Option<u32>,
    #[serde(default)]
    params: Option<serde_json::Value>,
}

/// A `NetworkResponse`-compatible reply. Chat/orchestrator decode replies
/// uniformly: `{success, output:{stdout, stderr, exit_status}, error}`. The
/// connector renders its JSON result document into `output.stdout` as text.
#[derive(Debug, Clone, Serialize)]
struct ConnectorReply {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<ConnectorReplyOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ConnectorReplyOutput {
    stdout: String,
    stderr: String,
    exit_status: i32,
}

impl ConnectorReply {
    fn ok(out: &ConnectorOutput) -> Self {
        let stdout = match &out.payload {
            serde_json::Value::String(s) => s.clone(),
            other => serde_json::to_string(other).unwrap_or_default(),
        };
        Self {
            success: true,
            output: Some(ConnectorReplyOutput {
                stdout,
                stderr: String::new(),
                exit_status: 0,
            }),
            error: None,
        }
    }
    fn err(msg: String) -> Self {
        Self {
            success: false,
            output: None,
            error: Some(msg),
        }
    }
}

/// Internal dispatch result carried between `dispatch` and the reply builders.
struct ConnectorOutput {
    capability: String,
    status: i32,
    payload: serde_json::Value,
}

#[async_trait]
impl Agent for ConnectorDriver {
    fn id(&self) -> &AgentId {
        &self.id
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    #[instrument(skip(self, ctx), fields(agent = %self.id))]
    async fn handle(&self, env: AgentEnvelope, ctx: AgentContext) -> Result<(), CoreError> {
        let req: ConnectorRequest = match serde_json::from_value(env.body.clone()) {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, "invalid connector request body");
                let reply = AgentEnvelope::reply_to(
                    &env,
                    self.id.clone(),
                    serde_json::to_value(ConnectorReply::err(format!("invalid request: {e}")))
                        .unwrap_or_default(),
                );
                return ctx.bus.send(reply).await;
            }
        };

        // FR-HAR-17: carry the CALLER identity from env.from as the audit/broker
        // actor — NOT the driver's own agent id.
        let actor_id = env.from.as_str();
        let timeout_secs = req.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
        let params = req.params.unwrap_or(serde_json::json!({}));
        debug!(capability = %req.capability, target = %req.target_ref, actor = %actor_id, "dispatch");

        let reply = match self
            .dispatch(actor_id, &req.capability, &req.target_ref, &params, timeout_secs)
            .await
        {
            Ok(out) => ConnectorReply::ok(&out),
            Err(e) => {
                error!(error = %e, "connector capability failed");
                ConnectorReply::err(e.to_string())
            }
        };
        let reply_env = AgentEnvelope::reply_to(
            &env,
            self.id.clone(),
            serde_json::to_value(reply).unwrap_or_default(),
        );
        ctx.bus.send(reply_env).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k8s_profile_toml() -> &'static str {
        include_str!("../../../deploy/connectors/k8s.toml")
    }

    fn parse_k8s() -> ConnectorProfile {
        toml::from_str(k8s_profile_toml()).expect("k8s.toml parses")
    }

    #[test]
    fn k8s_profile_projects_capabilities() {
        let p = parse_k8s();
        assert_eq!(p.class, ProfileClass::Orchestrator);
        assert_eq!(p.transport, ProfileTransport::Rest);
        let names: Vec<&str> = p.capabilities.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"orchestrator.k8s.pod.status"));
        assert!(names.contains(&"orchestrator.k8s.deploy.restart"));
        assert!(names.contains(&"orchestrator.k8s.deploy.rollback"));

        // Projected read cap is read-only; the writes are not.
        let status = p
            .capabilities
            .iter()
            .find(|c| c.name == "orchestrator.k8s.pod.status")
            .unwrap();
        assert!(project_capability(status).is_read(), "pod.status must be a read");

        let restart = p
            .capabilities
            .iter()
            .find(|c| c.name == "orchestrator.k8s.deploy.restart")
            .unwrap();
        let restart_cap = project_capability(restart);
        assert!(!restart_cap.is_read(), "deploy.restart must be a write");
        assert_eq!(
            restart_cap.compensating.as_ref().unwrap().name,
            "orchestrator.k8s.deploy.rollback"
        );
    }

    #[test]
    fn param_class_string_forms_parse() {
        let cases = [
            ("safe_text", ParamClass::SafeText),
            ("cidr", ParamClass::Cidr),
            ("identifier", ParamClass::Identifier),
            ("int", ParamClass::Int),
        ];
        for (s, expected) in cases {
            let toml_str = format!("v = \"{s}\"");
            #[derive(Deserialize)]
            struct W {
                v: ProfileParamClass,
            }
            let w: W = toml::from_str(&toml_str).unwrap();
            assert_eq!(w.v.0, expected);
        }
        // enum form
        #[derive(Deserialize)]
        struct W {
            v: ProfileParamClass,
        }
        let w: W = toml::from_str("v = \"enum:start,stop,reboot\"").unwrap();
        assert_eq!(
            w.v.0,
            ParamClass::Enum(vec!["start".into(), "stop".into(), "reboot".into()])
        );
        // unknown form rejected
        assert!(toml::from_str::<W>("v = \"nonsense\"").is_err());
    }

    #[test]
    fn substitute_fills_slots_and_rejects_missing() {
        let mut v = BTreeMap::new();
        v.insert("namespace".to_string(), "default".to_string());
        v.insert("name".to_string(), "web-0".to_string());
        let out = substitute(
            "/api/v1/namespaces/{namespace}/pods/{name}",
            "orchestrator.k8s.pod.status",
            &v,
        )
        .unwrap();
        assert_eq!(out, "/api/v1/namespaces/default/pods/web-0");

        // A slot with no validated value is a missing-param error — a template
        // can never emit an un-substituted `{slot}` into a URL.
        let err = substitute("/x/{ghost}", "c", &v).unwrap_err();
        assert!(matches!(err, ConnectorError::MissingParam { .. }));
    }

    #[test]
    fn render_ssh_op_substitutes_command() {
        // AC-P5-03: an SSH capability renders a validated Op::ShellCommand from
        // its command template.
        let op = ProfileOp {
            method: ProfileHttpMethod::Get,
            path: String::new(),
            body: None,
            command: Some("systemctl restart {service} && systemctl is-active {service}".into()),
            timeout_secs: None,
        };
        let mut v = BTreeMap::new();
        v.insert("service".to_string(), "nginx".to_string());
        match render_ssh_op(&op, "compute.host.service.restart", &v, 30).unwrap() {
            Op::ShellCommand {
                command,
                timeout_secs,
            } => {
                assert_eq!(command, "systemctl restart nginx && systemctl is-active nginx");
                assert_eq!(timeout_secs, 30);
            }
            other => panic!("expected ShellCommand, got {other:?}"),
        }
    }

    #[test]
    fn render_ssh_op_missing_command_errs() {
        let op = ProfileOp {
            method: ProfileHttpMethod::Get,
            path: String::new(),
            body: None,
            command: None,
            timeout_secs: None,
        };
        let err = render_ssh_op(&op, "c", &BTreeMap::new(), 30).unwrap_err();
        assert!(matches!(err, ConnectorError::BadProfile(_)));
    }

    #[test]
    fn injection_in_templated_param_is_rejected_before_op() {
        // The chokepoint: a crafted `namespace` value that would break the URL /
        // inject a path is rejected by param::validate (Identifier class) and no
        // Op is rendered. We drive the pure render path via the declared cap.
        let p = parse_k8s();
        let status = p
            .capabilities
            .iter()
            .find(|c| c.name == "orchestrator.k8s.pod.status")
            .unwrap();
        // namespace is declared `identifier`; a `/` or `;` violates it.
        let mut validated = BTreeMap::new();
        // simulate what dispatch does: validate first.
        let bad = "default/../../etc";
        let class = &status.params["namespace"].0;
        assert!(param::validate(bad, class).is_err(), "path traversal must be rejected");
        let semi = "default;rm";
        assert!(param::validate(semi, class).is_err(), "metachar must be rejected");
        // a clean value passes and renders.
        validated.insert("namespace".to_string(), "default".to_string());
        validated.insert("name".to_string(), "web-0".to_string());
        let op = render_http_op(&status.op, &status.name, &validated, &AuthScheme::Bearer).unwrap();
        match op {
            Op::Http { path, .. } => {
                assert_eq!(path, "/api/v1/namespaces/default/pods/web-0")
            }
            other => panic!("expected Http op, got {other:?}"),
        }
    }
}
