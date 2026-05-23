use serde::{Deserialize, Serialize};

use daimon_inventory::TargetRef;
use daimon_transport::Op;

/// What an agent sends to the broker.
///
/// `target_ref` selects the managed asset (inventory lookup → host + port +
/// transport + credential ref). `op` is the transport-level operation to
/// perform. `actor_id` identifies the calling agent for audit purposes —
/// workers set this to their own agent id; the admin UI uses `user:<username>`;
/// the orchestrator uses `orchestrator:<plan_id>`. The broker handles
/// credential resolution and dispatch internally; the agent never sees the
/// credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRequest {
    pub target_ref: TargetRef,
    pub op: Op,
    /// Caller identity recorded in the audit log (D23). Free-form string;
    /// convention is `agent:<id>` for worker agents, `user:<username>` for
    /// admin proxy callers, `orchestrator:<plan_id>` for plan steps.
    pub actor_id: String,
    /// Phase 5 — capability name being invoked, e.g.
    /// `"network.routeros.system_info"` or `"network.firewall.filter_add"`.
    /// When `Some`, the broker consults Guard's policy engine before
    /// dispatching. When `None`, the call bypasses policy (legacy /
    /// orchestrator-trusted path); KILL switch still fires unconditionally.
    #[serde(default)]
    pub capability: Option<String>,
    /// Read-only capabilities skip policy + approval (kill switch still
    /// applies). Set to `true` for read paths so the operator doesn't see
    /// approval prompts for harmless queries.
    #[serde(default)]
    pub is_read_only: bool,
}

impl ExecRequest {
    pub fn new(actor_id: impl Into<String>, target_ref: TargetRef, op: Op) -> Self {
        Self {
            target_ref,
            op,
            actor_id: actor_id.into(),
            capability: None,
            is_read_only: false,
        }
    }

    pub fn with_capability(mut self, capability: impl Into<String>, is_read_only: bool) -> Self {
        self.capability = Some(capability.into());
        self.is_read_only = is_read_only;
        self
    }
}
