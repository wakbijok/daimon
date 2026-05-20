use serde::{Deserialize, Serialize};

use daimon_inventory::TargetRef;
use daimon_transport::Op;

/// What an agent sends to the broker.
///
/// `target_ref` selects the managed asset (inventory lookup → host + port +
/// transport + credential ref). `op` is the transport-level operation to
/// perform. The broker handles credential resolution and dispatch internally;
/// the agent never sees the credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRequest {
    pub target_ref: TargetRef,
    pub op: Op,
}

impl ExecRequest {
    pub fn new(target_ref: TargetRef, op: Op) -> Self {
        Self { target_ref, op }
    }
}
