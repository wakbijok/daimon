use leptos::prelude::*;
use serde::{Deserialize, Serialize};

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

#[component]
pub fn NodeTable(rows: Vec<NodeRow>) -> impl IntoView {
    view! {
        <table class="w-full text-sm">
            <thead>
                <tr class="border-b border-border-primary text-text-muted text-[11px] uppercase tracking-wider">
                    <th class="text-left py-3 px-4 font-medium">"Node"</th>
                    <th class="text-left py-3 px-4 font-medium">"Status"</th>
                    <th class="text-left py-3 px-4 font-medium">"CPU"</th>
                    <th class="text-left py-3 px-4 font-medium">"Memory"</th>
                    <th class="text-left py-3 px-4 font-medium">"Disk"</th>
                    <th class="text-left py-3 px-4 font-medium">"Uptime"</th>
                </tr>
            </thead>
            <tbody>
                {rows.into_iter().map(|r| {
                    let online = r.status == "online";
                    let mem_pct = if r.mem_total > 0 { (r.mem_used as f64 / r.mem_total as f64) * 100.0 } else { 0.0 };
                    let disk_pct = if r.disk_total > 0 { (r.disk_used as f64 / r.disk_total as f64) * 100.0 } else { 0.0 };
                    view! {
                        <tr class="border-b border-border-primary/50 hover:bg-surface-tertiary/50">
                            <td class="py-3 px-4 text-text-primary font-medium">{r.name.clone()}</td>
                            <td class="py-3 px-4">
                                <span class="inline-flex items-center gap-1.5 text-[12px]">
                                    <span class=format!("w-2 h-2 rounded-full {}", if online { "bg-accent-green" } else { "bg-accent-danger" })></span>
                                    {if online { "Online" } else { "Offline" }}
                                </span>
                            </td>
                            <td class="py-3 px-4">
                                {pct_bar(r.cpu_pct, "bg-accent-green")}
                                <div class="text-text-muted text-[10px] mt-0.5">{format!("{:.0} vCPU", r.cpu_count)}</div>
                            </td>
                            <td class="py-3 px-4">
                                {pct_bar(mem_pct, "bg-accent-amber")}
                                <div class="text-text-muted text-[10px] mt-0.5">{format!("{} / {}", format_bytes(r.mem_used), format_bytes(r.mem_total))}</div>
                            </td>
                            <td class="py-3 px-4">
                                {pct_bar(disk_pct, "bg-accent-purple")}
                                <div class="text-text-muted text-[10px] mt-0.5">{format!("{} / {}", format_bytes(r.disk_used), format_bytes(r.disk_total))}</div>
                            </td>
                            <td class="py-3 px-4 text-text-secondary text-[13px]">{format_uptime(r.uptime)}</td>
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>
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

#[component]
pub fn GuestTable(rows: Vec<GuestRow>, guest_type: &'static str) -> impl IntoView {
    view! {
        <table class="w-full text-sm">
            <thead>
                <tr class="border-b border-border-primary text-text-muted text-[11px] uppercase tracking-wider">
                    <th class="text-left py-3 px-4 font-medium">"ID"</th>
                    <th class="text-left py-3 px-4 font-medium">"Name"</th>
                    <th class="text-left py-3 px-4 font-medium">"Node"</th>
                    <th class="text-left py-3 px-4 font-medium">"Status"</th>
                    <th class="text-left py-3 px-4 font-medium">"CPU"</th>
                    <th class="text-left py-3 px-4 font-medium">"Memory"</th>
                    <th class="text-left py-3 px-4 font-medium">"Disk"</th>
                    <th class="text-left py-3 px-4 font-medium">"Net I/O"</th>
                    <th class="text-left py-3 px-4 font-medium">"Uptime"</th>
                </tr>
            </thead>
            <tbody>
                {rows.into_iter().map(|r| {
                    let running = r.status == "running";
                    let mem_pct = if r.mem_total > 0 { (r.mem_used as f64 / r.mem_total as f64) * 100.0 } else { 0.0 };
                    view! {
                        <tr class=format!("border-b border-border-primary/50 hover:bg-surface-tertiary/50 {}", if !running { "opacity-50" } else { "" })>
                            <td class="py-3 px-4 text-text-muted text-[12px] font-mono">{r.vmid}</td>
                            <td class="py-3 px-4 text-text-primary font-medium">{r.name.clone()}</td>
                            <td class="py-3 px-4 text-text-muted text-[12px]">{r.node.clone()}</td>
                            <td class="py-3 px-4">
                                <span class="inline-flex items-center gap-1.5 text-[12px]">
                                    <span class=format!("w-2 h-2 rounded-full {}", if running { "bg-accent-green" } else { "bg-accent-danger" })></span>
                                    {r.status.clone()}
                                </span>
                            </td>
                            <td class="py-3 px-4">{pct_bar(r.cpu_pct, "bg-accent-green")}</td>
                            <td class="py-3 px-4">
                                {pct_bar(mem_pct, "bg-accent-amber")}
                                <div class="text-text-muted text-[10px] mt-0.5">{format!("{} / {}", format_bytes(r.mem_used), format_bytes(r.mem_total))}</div>
                            </td>
                            <td class="py-3 px-4 text-text-secondary text-[12px]">
                                {format!("{} / {}", format_bytes(r.disk_used), format_bytes(r.disk_total))}
                            </td>
                            <td class="py-3 px-4 text-text-muted text-[11px]">
                                <div>{"↓ "}{format_bytes(r.netin)}</div>
                                <div>{"↑ "}{format_bytes(r.netout)}</div>
                            </td>
                            <td class="py-3 px-4 text-text-secondary text-[13px]">{format_uptime(r.uptime)}</td>
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>

        <div class="mt-4 p-3 bg-surface-secondary border border-border-primary rounded-md text-text-muted text-xs flex items-center gap-2">
            <span class="text-accent-amber">"ℹ"</span>
            {format!("Install daimon-agent on your {}s for process-level metrics, actual memory usage, and service monitoring.", guest_type)}
        </div>
    }
}

// --- Storage ---

#[derive(Clone, Serialize, Deserialize)]
pub struct StorageRow {
    pub name: String,
    pub storage_type: String,
    pub content: String,
    pub used: u64,
    pub total: u64,
    pub avail: u64,
    pub shared: bool,
    pub active: bool,
}

#[component]
pub fn StorageTable(rows: Vec<StorageRow>) -> impl IntoView {
    view! {
        <table class="w-full text-sm">
            <thead>
                <tr class="border-b border-border-primary text-text-muted text-[11px] uppercase tracking-wider">
                    <th class="text-left py-3 px-4 font-medium">"Name"</th>
                    <th class="text-left py-3 px-4 font-medium">"Type"</th>
                    <th class="text-left py-3 px-4 font-medium">"Content"</th>
                    <th class="text-left py-3 px-4 font-medium">"Usage"</th>
                    <th class="text-left py-3 px-4 font-medium">"Total"</th>
                    <th class="text-left py-3 px-4 font-medium">"Available"</th>
                    <th class="text-right py-3 px-4 font-medium">"Status"</th>
                </tr>
            </thead>
            <tbody>
                {rows.into_iter().map(|r| {
                    let used_pct = if r.total > 0 { (r.used as f64 / r.total as f64) * 100.0 } else { 0.0 };
                    view! {
                        <tr class="border-b border-border-primary/50 hover:bg-surface-tertiary/50">
                            <td class="py-3 px-4 text-text-primary font-medium">{r.name.clone()}</td>
                            <td class="py-3 px-4 text-text-muted text-[12px]">{r.storage_type.clone()}</td>
                            <td class="py-3 px-4 text-text-muted text-[12px]">{r.content.clone()}</td>
                            <td class="py-3 px-4">{pct_bar(used_pct, "bg-accent-purple")}</td>
                            <td class="py-3 px-4 text-text-secondary text-[12px]">{format_bytes(r.total)}</td>
                            <td class="py-3 px-4 text-text-secondary text-[12px]">{format_bytes(r.avail)}</td>
                            <td class="py-3 px-4 text-right">
                                <span class="inline-flex items-center gap-1.5 text-[12px]">
                                    <span class=format!("w-2 h-2 rounded-full {}", if r.active { "bg-accent-green" } else { "bg-accent-danger" })></span>
                                    {if r.active { "Active" } else { "Inactive" }}
                                </span>
                            </td>
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>
    }
}
