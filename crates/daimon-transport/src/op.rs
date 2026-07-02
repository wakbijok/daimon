use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// A transport-level operation.
///
/// Worker agents construct these (through `daimon-broker::ExecRequest`) but
/// never execute them directly — the broker dispatches via this crate using
/// resolved credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Op {
    /// Execute a shell command over SSH. `command` is the raw command string;
    /// Phase 2 deliberately does not parse or escape — that's the worker's
    /// responsibility (with Guard policy gating mutations).
    ShellCommand {
        command: String,
        #[serde(default = "default_shell_timeout_secs")]
        timeout_secs: u32,
    },
    /// HTTP/REST request.
    Http {
        method: HttpMethod,
        path: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        headers: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<serde_json::Value>,
    },
    /// SNMP GET — returns a value.
    SnmpGet { oid: String },
    /// SNMP SET — sets a value.
    SnmpSet { oid: String, value: SnmpValue },
    /// SNMP WALK — iterates a subtree.
    SnmpWalk { oid_root: String },
}

fn default_shell_timeout_secs() -> u32 {
    30
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SnmpValue {
    Int(i64),
    String(String),
    Oid(String),
}

/// Result of a transport operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpResult {
    /// Shell command result.
    ShellCommand {
        stdout: String,
        stderr: String,
        exit_status: i32,
    },
    /// HTTP response.
    Http {
        status: u16,
        body: serde_json::Value,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        headers: BTreeMap<String, String>,
    },
    /// SNMP value(s).
    Snmp { values: BTreeMap<String, SnmpValue> },
    /// SNMP SET acknowledgement.
    SnmpSetAck,
    /// A driver-parsed typed document (FR-CON-16). Lets `read_state`/`diagnose`
    /// return a typed `StateDoc`/`Finding` without regexing stdout — a transport
    /// (or a driver post-processing a raw result) can hand back structure
    /// directly.
    Structured { doc: serde_json::Value },
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("transport `{kind}` not implemented yet")]
    NotImplemented { kind: String },
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("connection failed: {0}")]
    Connect(String),
    #[error("operation timed out after {0}s")]
    Timeout(u32),
    #[error("transport I/O: {0}")]
    Io(String),
    #[error("op/transport mismatch: cannot run `{op}` over `{transport}`")]
    OpMismatch { op: String, transport: String },
    #[error("{0}")]
    Other(String),
}
