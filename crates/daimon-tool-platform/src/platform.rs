//! Platform trait + workload types.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadKind {
    Vm,
    Container,
    Node,
    Storage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workload {
    pub id: String,
    pub name: String,
    pub kind: WorkloadKind,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(default)]
    pub cpu_pct: f32,
    #[serde(default)]
    pub cpu_count: f64,
    #[serde(default)]
    pub mem_used: u64,
    #[serde(default)]
    pub mem_total: u64,
    #[serde(default)]
    pub disk_used: u64,
    #[serde(default)]
    pub disk_total: u64,
    #[serde(default)]
    pub uptime: u64,
    /// Platform-specific extra fields the caller can pick from. Common
    /// values: netin/netout, host (for VMs), template (bool), etc.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub name: String,
    pub workload_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub parent: Option<String>,
    pub description: Option<String>,
}

/// Capability-based core. Every Platform impl supports list + get.
#[async_trait]
pub trait Platform: Send + Sync {
    /// Cluster id — distinguishes multiple instances of the same kind
    /// (e.g. "pve-bijok01" vs "pve-prod").
    fn id(&self) -> &str;

    /// "pve", "kubernetes", "openstack", etc.
    fn kind(&self) -> &'static str;

    /// All workloads across all nodes. Caller may filter by kind via the
    /// returned `Workload.kind` field.
    async fn list_workloads(&self) -> Result<Vec<Workload>>;

    /// Single workload by id (`<vmid>` for PVE).
    async fn get_workload(&self, id: &str) -> Result<Option<Workload>>;
}

#[async_trait]
pub trait Snapshotable: Send + Sync {
    async fn snapshot(&self, workload_id: &str, name: &str, description: Option<&str>) -> Result<()>;
    async fn list_snapshots(&self, workload_id: &str) -> Result<Vec<Snapshot>>;
    async fn delete_snapshot(&self, workload_id: &str, name: &str) -> Result<()>;
}

#[async_trait]
pub trait Cloneable: Send + Sync {
    /// Clone a workload. `new_id` is platform-assigned if `None`; caller
    /// may suggest one. Returns the resulting workload's id.
    async fn clone_workload(
        &self,
        workload_id: &str,
        new_name: &str,
        new_id: Option<&str>,
    ) -> Result<String>;
}
