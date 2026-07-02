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
    /// NON-AUTHORITATIVE UX hint only. The broker no longer trusts this for
    /// the guard decision — it derives read-only server-side from
    /// `capability_meta` (the H6/H7 fix). Kept for callers that carry only a
    /// name and for display.
    #[serde(default)]
    pub is_read_only: bool,
    /// The resolved capability descriptor. When present, the broker derives
    /// the guard's read-only disposition from `Capability::is_read()` — the
    /// server-side authority. `None` is treated as a WRITE (fail-closed), so a
    /// capability-less request cannot skip policy. P2 will populate this from
    /// the CapabilityRegistry; today workers attach their own catalog entry.
    #[serde(default)]
    pub capability_meta: Option<daimon_core::Capability>,
}

impl ExecRequest {
    pub fn new(actor_id: impl Into<String>, target_ref: TargetRef, op: Op) -> Self {
        Self {
            target_ref,
            op,
            actor_id: actor_id.into(),
            capability: None,
            is_read_only: false,
            capability_meta: None,
        }
    }

    /// Attach a capability by NAME only (no server-side disposition). The
    /// broker treats this as a write unless `capability_meta` is also set.
    /// `is_read_only` here is a non-authoritative hint.
    pub fn with_capability(mut self, capability: impl Into<String>, is_read_only: bool) -> Self {
        self.capability = Some(capability.into());
        self.is_read_only = is_read_only;
        self
    }

    /// Attach the resolved `Capability` descriptor — the authoritative source
    /// for the broker's read-only derivation. Sets the capability name too.
    pub fn with_capability_meta(mut self, capability: daimon_core::Capability) -> Self {
        self.is_read_only = capability.is_read();
        self.capability = Some(capability.name.clone());
        self.capability_meta = Some(capability);
        self
    }
}
