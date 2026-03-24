use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct PveVersion {
    pub version: String,
    pub release: String,
    pub repoid: String,
}

/// Cluster resource — returned by /cluster/resources
/// PVE returns mixed types (node, qemu, lxc, storage) in one response
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// RRD time-series data point — returned by /nodes/{node}/rrddata
/// Note: All numeric values are f64 (PVE RRD returns floats), unlike PveResource which uses u64 for mem/disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RrdDataPoint {
    pub time: f64,
    #[serde(default)]
    pub cpu: Option<f64>,
    #[serde(default)]
    pub maxcpu: Option<f64>,
    #[serde(default)]
    pub mem: Option<f64>,
    #[serde(default)]
    pub maxmem: Option<f64>,
    #[serde(default)]
    pub disk: Option<f64>,
    #[serde(default)]
    pub maxdisk: Option<f64>,
    #[serde(default)]
    pub netin: Option<f64>,
    #[serde(default)]
    pub netout: Option<f64>,
    #[serde(default)]
    pub diskread: Option<f64>,
    #[serde(default)]
    pub diskwrite: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum RrdTimeframe {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

impl RrdTimeframe {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaStatus {
    #[serde(default)]
    pub managed: Option<u8>,
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QemuStatus {
    #[serde(default)]
    pub pid: Option<u64>,
    #[serde(default)]
    pub qmpstatus: Option<String>,
    #[serde(default, rename = "running-machine")]
    pub running_machine: Option<String>,
    #[serde(default, rename = "running-qemu")]
    pub running_qemu: Option<String>,
    #[serde(default)]
    pub ha: Option<HaStatus>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub cpu: Option<f64>,
    #[serde(default)]
    pub maxcpu: Option<u64>,
    #[serde(default)]
    pub mem: Option<u64>,
    #[serde(default)]
    pub maxmem: Option<u64>,
    #[serde(default)]
    pub disk: Option<u64>,
    #[serde(default)]
    pub maxdisk: Option<u64>,
    #[serde(default)]
    pub netin: Option<u64>,
    #[serde(default)]
    pub netout: Option<u64>,
    #[serde(default)]
    pub uptime: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LxcStatus {
    #[serde(default)]
    pub pid: Option<u64>,
    #[serde(default)]
    pub ha: Option<HaStatus>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub cpu: Option<f64>,
    #[serde(default)]
    pub maxcpu: Option<u64>,
    #[serde(default)]
    pub mem: Option<u64>,
    #[serde(default)]
    pub maxmem: Option<u64>,
    #[serde(default)]
    pub disk: Option<u64>,
    #[serde(default)]
    pub maxdisk: Option<u64>,
    #[serde(default)]
    pub netin: Option<u64>,
    #[serde(default)]
    pub netout: Option<u64>,
    #[serde(default)]
    pub uptime: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
}

/// VM/LXC configuration — PVE returns numbered keys (net0, scsi0, etc.) as flat JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuestConfig {
    #[serde(default)]
    pub cores: Option<u32>,
    #[serde(default)]
    pub memory: Option<u64>,
    #[serde(default)]
    pub balloon: Option<u64>,
    #[serde(default)]
    pub sockets: Option<u32>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub ostype: Option<String>,
    #[serde(skip)]
    pub net_devices: Vec<String>,
    #[serde(skip)]
    pub disk_devices: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrd_data_point_deserializes_from_pve_json() {
        let json = r#"{"time":1711296000,"cpu":0.0523,"maxcpu":8,"mem":12345678,"maxmem":33554432}"#;
        let point: RrdDataPoint = serde_json::from_str(json).unwrap();
        assert_eq!(point.time, 1711296000.0);
        assert!((point.cpu.unwrap() - 0.0523).abs() < f64::EPSILON);
        assert_eq!(point.maxcpu.unwrap(), 8.0);
    }

    #[test]
    fn rrd_data_point_handles_missing_fields() {
        let json = r#"{"time":1711296000}"#;
        let point: RrdDataPoint = serde_json::from_str(json).unwrap();
        assert!(point.cpu.is_none());
        assert!(point.mem.is_none());
    }

    #[test]
    fn rrd_timeframe_as_str() {
        assert_eq!(RrdTimeframe::Hour.as_str(), "hour");
        assert_eq!(RrdTimeframe::Year.as_str(), "year");
    }

    #[test]
    fn qemu_status_deserializes_with_ha() {
        let json = r#"{"status":"running","cpu":0.05,"maxcpu":4,"mem":1073741824,"maxmem":4294967296,"uptime":86400,"ha":{"managed":1,"state":"started"},"pid":12345}"#;
        let status: QemuStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status.status, "running");
        assert_eq!(status.pid, Some(12345));
        assert!(status.ha.is_some());
        assert_eq!(status.ha.unwrap().state, Some("started".to_string()));
    }
}
