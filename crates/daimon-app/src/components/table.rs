use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use crate::components::sortable_table::{TableRow, ColumnDef, SortType};

pub fn format_bytes(bytes: u64) -> String {
    if bytes == 0 { return "0 B".to_string(); }
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut val = bytes as f64;
    let mut unit = 0;
    while val >= 1024.0 && unit < units.len() - 1 {
        val /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{} B", bytes) }
    else { format!("{:.1} {}", val, units[unit]) }
}

pub fn format_uptime(secs: u64) -> String {
    if secs == 0 { return "-".to_string(); }
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 { format!("{}d {}h", days, hours) }
    else if hours > 0 { format!("{}h {}m", hours, mins) }
    else { format!("{}m", mins) }
}

fn pct_bar(pct: f64, color: &str) -> impl IntoView {
    let width = format!("width:{}%", pct.min(100.0));
    let bar_class = format!("h-1 rounded-full {}", color);
    view! {
        <div class="flex items-center gap-2">
            <span class="text-[13px] w-14 text-right font-mono">{format!("{:.1}%", pct)}</span>
            <div class="flex-1 h-1 bg-surface-tertiary rounded-full max-w-24">
                <div class=bar_class style=width></div>
            </div>
        </div>
    }
}

// --- Node ---

#[derive(Clone, Serialize, Deserialize)]
pub struct NodeRow {
    pub name: String,
    pub status: String,
    pub cpu_pct: f64,
    pub cpu_count: f64,
    pub mem_used: u64,
    pub mem_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    pub uptime: u64,
}

impl TableRow for NodeRow {
    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef { key: "name", label: "Node", sortable: true, default_hidden: false, sort_type: SortType::Text },
            ColumnDef { key: "status", label: "Status", sortable: true, default_hidden: false, sort_type: SortType::Text },
            ColumnDef { key: "cpu", label: "CPU", sortable: true, default_hidden: false, sort_type: SortType::Percentage },
            ColumnDef { key: "memory", label: "Memory", sortable: true, default_hidden: false, sort_type: SortType::Percentage },
            ColumnDef { key: "disk", label: "Disk", sortable: true, default_hidden: false, sort_type: SortType::Percentage },
            ColumnDef { key: "uptime", label: "Uptime", sortable: true, default_hidden: false, sort_type: SortType::Numeric },
        ]
    }

    fn cell_value(&self, col: &str) -> String {
        match col {
            "name" => self.name.clone(),
            "status" => self.status.clone(),
            "cpu" => format!("{:.2}", self.cpu_pct),
            "memory" => {
                let pct = if self.mem_total > 0 { (self.mem_used as f64 / self.mem_total as f64) * 100.0 } else { 0.0 };
                format!("{:.2}", pct)
            }
            "disk" => {
                let pct = if self.disk_total > 0 { (self.disk_used as f64 / self.disk_total as f64) * 100.0 } else { 0.0 };
                format!("{:.2}", pct)
            }
            "uptime" => self.uptime.to_string(),
            _ => String::new(),
        }
    }

    fn cell_view(&self, col: &str) -> AnyView {
        match col {
            "name" => view! {
                <span class="text-text-primary font-medium">{self.name.clone()}</span>
            }.into_any(),
            "status" => {
                let online = self.status == "online";
                view! {
                    <span class="inline-flex items-center gap-1.5 text-[12px]">
                        <span class=format!("w-2 h-2 rounded-full {}", if online { "bg-accent-green" } else { "bg-accent-danger" })></span>
                        {if online { "Online" } else { "Offline" }}
                    </span>
                }.into_any()
            }
            "cpu" => view! {
                <div>
                    {pct_bar(self.cpu_pct, "bg-accent-green")}
                    <div class="text-text-muted text-[10px] mt-0.5">{format!("{:.0} vCPU", self.cpu_count)}</div>
                </div>
            }.into_any(),
            "memory" => {
                let mem_pct = if self.mem_total > 0 { (self.mem_used as f64 / self.mem_total as f64) * 100.0 } else { 0.0 };
                let mem_used = self.mem_used;
                let mem_total = self.mem_total;
                view! {
                    <div>
                        {pct_bar(mem_pct, "bg-accent-amber")}
                        <div class="text-text-muted text-[10px] mt-0.5">{format!("{} / {}", format_bytes(mem_used), format_bytes(mem_total))}</div>
                    </div>
                }.into_any()
            }
            "disk" => {
                let disk_pct = if self.disk_total > 0 { (self.disk_used as f64 / self.disk_total as f64) * 100.0 } else { 0.0 };
                let disk_used = self.disk_used;
                let disk_total = self.disk_total;
                view! {
                    <div>
                        {pct_bar(disk_pct, "bg-accent-purple")}
                        <div class="text-text-muted text-[10px] mt-0.5">{format!("{} / {}", format_bytes(disk_used), format_bytes(disk_total))}</div>
                    </div>
                }.into_any()
            }
            "uptime" => view! {
                <span class="text-text-secondary text-[13px]">{format_uptime(self.uptime)}</span>
            }.into_any(),
            _ => view! {}.into_any(),
        }
    }

    fn row_key(&self) -> String {
        self.name.clone()
    }
}

// --- Guest (VM / LXC) ---

#[derive(Clone, Serialize, Deserialize)]
pub struct GuestRow {
    pub vmid: u32,
    pub name: String,
    pub node: String,
    pub status: String,
    pub cpu_pct: f64,
    pub cpu_count: f64,
    pub mem_used: u64,
    pub mem_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    pub netin: u64,
    pub netout: u64,
    pub uptime: u64,
}

impl TableRow for GuestRow {
    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef { key: "id", label: "ID", sortable: true, default_hidden: false, sort_type: SortType::Numeric },
            ColumnDef { key: "name", label: "Name", sortable: true, default_hidden: false, sort_type: SortType::Text },
            ColumnDef { key: "node", label: "Node", sortable: true, default_hidden: false, sort_type: SortType::Text },
            ColumnDef { key: "status", label: "Status", sortable: true, default_hidden: false, sort_type: SortType::Text },
            ColumnDef { key: "cpu", label: "CPU", sortable: true, default_hidden: false, sort_type: SortType::Percentage },
            ColumnDef { key: "memory", label: "Memory", sortable: true, default_hidden: false, sort_type: SortType::Percentage },
            ColumnDef { key: "disk", label: "Disk", sortable: true, default_hidden: false, sort_type: SortType::Text },
            ColumnDef { key: "net_io", label: "Net I/O", sortable: true, default_hidden: false, sort_type: SortType::Text },
            ColumnDef { key: "uptime", label: "Uptime", sortable: true, default_hidden: false, sort_type: SortType::Numeric },
        ]
    }

    fn cell_value(&self, col: &str) -> String {
        match col {
            "id" => self.vmid.to_string(),
            "name" => self.name.clone(),
            "node" => self.node.clone(),
            "status" => self.status.clone(),
            "cpu" => format!("{:.2}", self.cpu_pct),
            "memory" => {
                let pct = if self.mem_total > 0 { (self.mem_used as f64 / self.mem_total as f64) * 100.0 } else { 0.0 };
                format!("{:.2}", pct)
            }
            "disk" => format!("{} / {}", format_bytes(self.disk_used), format_bytes(self.disk_total)),
            "net_io" => format!("{} / {}", format_bytes(self.netin), format_bytes(self.netout)),
            "uptime" => self.uptime.to_string(),
            _ => String::new(),
        }
    }

    fn cell_view(&self, col: &str) -> AnyView {
        match col {
            "id" => view! {
                <span class="text-text-muted text-[12px] font-mono">{self.vmid}</span>
            }.into_any(),
            "name" => view! {
                <span class="text-text-primary font-medium">{self.name.clone()}</span>
            }.into_any(),
            "node" => view! {
                <span class="text-text-muted text-[12px]">{self.node.clone()}</span>
            }.into_any(),
            "status" => {
                let running = self.status == "running";
                let status = self.status.clone();
                view! {
                    <span class="inline-flex items-center gap-1.5 text-[12px]">
                        <span class=format!("w-2 h-2 rounded-full {}", if running { "bg-accent-green" } else { "bg-accent-danger" })></span>
                        {status}
                    </span>
                }.into_any()
            }
            "cpu" => view! {
                <div>{pct_bar(self.cpu_pct, "bg-accent-green")}</div>
            }.into_any(),
            "memory" => {
                let mem_pct = if self.mem_total > 0 { (self.mem_used as f64 / self.mem_total as f64) * 100.0 } else { 0.0 };
                let mem_used = self.mem_used;
                let mem_total = self.mem_total;
                view! {
                    <div>
                        {pct_bar(mem_pct, "bg-accent-amber")}
                        <div class="text-text-muted text-[10px] mt-0.5">{format!("{} / {}", format_bytes(mem_used), format_bytes(mem_total))}</div>
                    </div>
                }.into_any()
            }
            "disk" => {
                let disk_used = self.disk_used;
                let disk_total = self.disk_total;
                view! {
                    <span class="text-text-secondary text-[12px]">
                        {format!("{} / {}", format_bytes(disk_used), format_bytes(disk_total))}
                    </span>
                }.into_any()
            }
            "net_io" => {
                let netin = self.netin;
                let netout = self.netout;
                view! {
                    <div class="text-text-muted text-[11px]">
                        <div>{format!("\u{2193} {}", format_bytes(netin))}</div>
                        <div>{format!("\u{2191} {}", format_bytes(netout))}</div>
                    </div>
                }.into_any()
            }
            "uptime" => view! {
                <span class="text-text-secondary text-[13px]">{format_uptime(self.uptime)}</span>
            }.into_any(),
            _ => view! {}.into_any(),
        }
    }

    fn row_key(&self) -> String {
        self.vmid.to_string()
    }
}

// --- Storage ---

#[derive(Clone, Serialize, Deserialize)]
pub struct StorageRow {
    pub name: String,
    pub node: String,
    pub storage_type: String,
    pub content: String,
    pub used: u64,
    pub total: u64,
    pub avail: u64,
    pub shared: bool,
    pub active: bool,
}

impl TableRow for StorageRow {
    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef { key: "name", label: "Name", sortable: true, default_hidden: false, sort_type: SortType::Text },
            ColumnDef { key: "node", label: "Node", sortable: true, default_hidden: false, sort_type: SortType::Text },
            ColumnDef { key: "type", label: "Type", sortable: true, default_hidden: false, sort_type: SortType::Text },
            ColumnDef { key: "content", label: "Content", sortable: true, default_hidden: false, sort_type: SortType::Text },
            ColumnDef { key: "usage", label: "Usage", sortable: true, default_hidden: false, sort_type: SortType::Percentage },
            ColumnDef { key: "total", label: "Total", sortable: true, default_hidden: false, sort_type: SortType::Numeric },
            ColumnDef { key: "available", label: "Available", sortable: true, default_hidden: false, sort_type: SortType::Numeric },
            ColumnDef { key: "status", label: "Status", sortable: true, default_hidden: false, sort_type: SortType::Text },
        ]
    }

    fn cell_value(&self, col: &str) -> String {
        match col {
            "name" => self.name.clone(),
            "node" => self.node.clone(),
            "type" => self.storage_type.clone(),
            "content" => self.content.clone(),
            "usage" => {
                let pct = if self.total > 0 { (self.used as f64 / self.total as f64) * 100.0 } else { 0.0 };
                format!("{:.2}", pct)
            }
            "total" => self.total.to_string(),
            "available" => self.avail.to_string(),
            "status" => if self.active { "Active".to_string() } else { "Inactive".to_string() },
            _ => String::new(),
        }
    }

    fn cell_view(&self, col: &str) -> AnyView {
        match col {
            "name" => view! {
                <span class="text-text-primary font-medium">{self.name.clone()}</span>
            }.into_any(),
            "node" => view! {
                <span class="text-text-muted text-[12px]">{self.node.clone()}</span>
            }.into_any(),
            "type" => view! {
                <span class="text-text-muted text-[12px]">{self.storage_type.clone()}</span>
            }.into_any(),
            "content" => view! {
                <span class="text-text-muted text-[12px]">{self.content.clone()}</span>
            }.into_any(),
            "usage" => {
                let used_pct = if self.total > 0 { (self.used as f64 / self.total as f64) * 100.0 } else { 0.0 };
                view! {
                    <div>{pct_bar(used_pct, "bg-accent-purple")}</div>
                }.into_any()
            }
            "total" => view! {
                <span class="text-text-secondary text-[12px]">{format_bytes(self.total)}</span>
            }.into_any(),
            "available" => view! {
                <span class="text-text-secondary text-[12px]">{format_bytes(self.avail)}</span>
            }.into_any(),
            "status" => {
                let active = self.active;
                view! {
                    <span class="inline-flex items-center gap-1.5 text-[12px]">
                        <span class=format!("w-2 h-2 rounded-full {}", if active { "bg-accent-green" } else { "bg-accent-danger" })></span>
                        {if active { "Active" } else { "Inactive" }}
                    </span>
                }.into_any()
            }
            _ => view! {}.into_any(),
        }
    }

    fn row_key(&self) -> String {
        format!("{}:{}", self.node, self.name)
    }
}
