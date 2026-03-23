use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct PveVersion {
    pub version: String,
    pub release: String,
    pub repoid: String,
}

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
