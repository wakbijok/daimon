//! PVE Platform driver — wraps `daimon-pve::Client` behind the Platform
//! trait surface. Phase 7 ships read-only paths; snapshot + clone (write
//! capabilities) land in Phase 7.1 once Guard-gated via broker.

use async_trait::async_trait;
use daimon_pve::Client;
use serde_json::json;

use crate::error::{Error, Result};
use crate::platform::{Platform, Workload, WorkloadKind};

pub struct PveDriver {
    id: String,
    client: Client,
}

impl PveDriver {
    pub fn new(cluster_id: impl Into<String>, client: Client) -> Self {
        Self {
            id: cluster_id.into(),
            client,
        }
    }

    /// Borrow the underlying PVE client — used by daimon-app paths that
    /// still need the rich client surface (RRD queries etc.) during the
    /// Phase 7 migration window.
    pub fn client(&self) -> &Client {
        &self.client
    }
}

#[async_trait]
impl Platform for PveDriver {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &'static str {
        "pve"
    }

    async fn list_workloads(&self) -> Result<Vec<Workload>> {
        let resources = self.client.cluster_resources(None).await?;
        Ok(resources
            .into_iter()
            .filter_map(|r| Some(resource_to_workload(r)?))
            .collect())
    }

    async fn get_workload(&self, id: &str) -> Result<Option<Workload>> {
        let vmid: u32 = id.parse().map_err(|_| Error::Other(format!("bad vmid `{id}`")))?;
        let resources = self.client.cluster_resources(None).await?;
        Ok(resources
            .into_iter()
            .find(|r| r.vmid == Some(vmid))
            .and_then(resource_to_workload))
    }
}

fn resource_to_workload(r: daimon_pve::PveResource) -> Option<Workload> {
    let kind = match r.resource_type.as_str() {
        "qemu" => WorkloadKind::Vm,
        "lxc" => WorkloadKind::Container,
        "node" => WorkloadKind::Node,
        "storage" => WorkloadKind::Storage,
        _ => return None,
    };
    let id = r
        .vmid
        .map(|v| v.to_string())
        .or_else(|| r.storage.clone())
        .unwrap_or_else(|| r.node.clone());
    let metadata = json!({
        "netin": r.netin,
        "netout": r.netout,
        "type": r.resource_type,
        "shared": r.shared,
        "plugintype": r.plugintype,
        "content": r.content,
    });
    Some(Workload {
        id,
        name: r.name.clone(),
        kind,
        status: r.status.clone(),
        node: if r.node.is_empty() { None } else { Some(r.node.clone()) },
        cpu_pct: (r.cpu * 100.0) as f32,
        cpu_count: r.maxcpu,
        mem_used: r.mem,
        mem_total: r.maxmem,
        disk_used: r.disk,
        disk_total: r.maxdisk,
        uptime: r.uptime,
        metadata,
    })
}
