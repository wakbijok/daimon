use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct PveVersion {
    pub version: String,
    pub release: String,
    pub repoid: String,
}

/// Cluster resource — returned by /cluster/resources
/// PVE returns mixed types (node, qemu, lxc, storage) in one response
#[derive(Debug, Clone, Deserialize)]
pub struct PveResource {
    pub id: String,
    #[serde(rename = "type")]
    pub resource_type: String,
    #[serde(default)]
    pub node: String,
    #[serde(default)]
    pub status: String,

    // Compute metrics (nodes, VMs, LXCs)
    #[serde(default)]
    pub cpu: f64,
    #[serde(default)]
    pub maxcpu: f64,
    #[serde(default)]
    pub mem: u64,
    #[serde(default)]
    pub maxmem: u64,

    // Disk (nodes use disk/maxdisk, VMs/LXCs also)
    #[serde(default)]
    pub disk: u64,
    #[serde(default)]
    pub maxdisk: u64,

    // Network I/O (VMs, LXCs)
    #[serde(default)]
    pub netin: u64,
    #[serde(default)]
    pub netout: u64,

    // Disk I/O (VMs, LXCs)
    #[serde(default)]
    pub diskread: u64,
    #[serde(default)]
    pub diskwrite: u64,

    // VM/LXC specific
    #[serde(default)]
    pub vmid: Option<u32>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub uptime: u64,
    #[serde(default)]
    pub template: Option<u8>,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub hastate: Option<String>,
    #[serde(default)]
    pub lock: Option<String>,

    // Storage specific
    #[serde(default)]
    pub storage: Option<String>,
    #[serde(default)]
    pub plugintype: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub shared: Option<u8>,
}

/// Node status — detailed info from /nodes/{node}/status
#[derive(Debug, Clone, Deserialize)]
pub struct PveNodeStatus {
    #[serde(default)]
    pub uptime: u64,
    #[serde(default)]
    pub loadavg: Vec<String>,
    #[serde(default)]
    pub cpuinfo: Option<PveCpuInfo>,
    #[serde(default)]
    pub memory: Option<PveMemInfo>,
    #[serde(default)]
    pub rootfs: Option<PveDiskInfo>,
    #[serde(default)]
    pub swap: Option<PveMemInfo>,
    #[serde(default)]
    pub kversion: Option<String>,
    #[serde(default)]
    pub pveversion: Option<String>,
    #[serde(default)]
    pub cpu: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PveCpuInfo {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub cores: u32,
    #[serde(default)]
    pub sockets: u32,
    #[serde(default)]
    pub cpus: u32,
    #[serde(default)]
    pub mhz: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PveMemInfo {
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub used: u64,
    #[serde(default)]
    pub free: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PveDiskInfo {
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub used: u64,
    #[serde(default)]
    pub free: u64,
    #[serde(default)]
    pub avail: u64,
}

// Keep the old simple types for backwards compat with existing tests
#[derive(Debug, Clone, Deserialize)]
pub struct PveNode {
    pub node: String,
    pub status: String,
    #[serde(default)]
    pub cpu: f64,
    #[serde(default)]
    pub maxcpu: u32,
    #[serde(default)]
    pub mem: u64,
    #[serde(default)]
    pub maxmem: u64,
    #[serde(default)]
    pub uptime: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PveVm {
    pub vmid: u32,
    #[serde(default)]
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub cpu: f64,
    #[serde(default)]
    pub cpus: u32,
    #[serde(default)]
    pub mem: u64,
    #[serde(default)]
    pub maxmem: u64,
    #[serde(default)]
    pub uptime: u64,
    #[serde(default)]
    pub node: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PveLxc {
    pub vmid: u32,
    #[serde(default)]
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub cpu: f64,
    #[serde(default)]
    pub cpus: u32,
    #[serde(default)]
    pub mem: u64,
    #[serde(default)]
    pub maxmem: u64,
    #[serde(default)]
    pub uptime: u64,
    #[serde(default)]
    pub node: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PveStorage {
    pub storage: String,
    #[serde(rename = "type")]
    pub storage_type: String,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub used: u64,
    #[serde(default)]
    pub avail: u64,
    #[serde(default)]
    pub active: Option<u8>,
    #[serde(default)]
    pub content: String,
}
