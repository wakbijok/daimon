use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use leptos_router::components::Outlet;
use crate::components::detail_layout::{DetailLayout, DetailTab};
use crate::components::summary_bar::{SummaryBar, SummaryItem};
use crate::components::sparkline::Sparkline;
use crate::components::table::{format_bytes, format_uptime};
use super::detail::{RrdPoint, AppGuestConfig};

#[component]
pub fn VmDetail() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let vmid = move || params.get().get("vmid").unwrap_or_default();
    let base = move || format!("/clusters/{}/vms/{}", cluster_id(), vmid());

    view! {
        <DetailLayout
            title=format!("VM {}", vmid())
            subtitle="QEMU Virtual Machine".to_string()
            tabs=vec![
                DetailTab { label: "Overview", path: base(), requires_agent: false },
                DetailTab { label: "Processes", path: format!("{}/processes", base()), requires_agent: true },
                DetailTab { label: "Services", path: format!("{}/services", base()), requires_agent: true },
                DetailTab { label: "Network", path: format!("{}/network", base()), requires_agent: true },
                DetailTab { label: "Logs", path: format!("{}/logs", base()), requires_agent: true },
                DetailTab { label: "Charts", path: format!("{}/charts", base()), requires_agent: false },
            ]
        >
            <Outlet />
        </DetailLayout>
    }
}

#[component]
pub fn VmOverview() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let vmid_str = move || params.get().get("vmid").unwrap_or_default();
    let vmid = move || vmid_str().parse::<u32>().unwrap_or(0);

    // First, find which node this VM is on
    let node_info = Resource::new(
        move || (cluster_id(), vmid()),
        |(cid, vid)| super::detail::find_guest_node(cid, vid),
    );

    view! {
        <Suspense fallback=|| view! { <p class="text-text-muted text-sm">"Locating VM..."</p> }>
            {move || node_info.get().map(|result| match result {
                Ok((node_name, _guest_type)) => {
                    let cid = cluster_id();
                    let vid = vmid();
                    let node = node_name.clone();
                    view! {
                        <GuestOverviewInner
                            cluster_id=cid
                            node=node
                            vmid=vid
                            guest_type="qemu".to_string()
                        />
                    }.into_any()
                }
                Err(e) => view! {
                    <p class="text-accent-danger text-sm">{format!("Error: {}", e)}</p>
                }.into_any(),
            })}
        </Suspense>
    }
}

/// Shared inner component for VM and LXC overviews, parameterised by guest_type
#[component]
pub fn GuestOverviewInner(
    cluster_id: String,
    node: String,
    vmid: u32,
    guest_type: String,
) -> impl IntoView {
    let cid = cluster_id.clone();
    let cid2 = cluster_id.clone();
    let cid3 = cluster_id.clone();
    let node1 = node.clone();
    let node2 = node.clone();
    let node3 = node.clone();
    let gt1 = guest_type.clone();
    let gt2 = guest_type.clone();
    let gt3 = guest_type.clone();
    let node_display = node.clone();

    // Fetch guest status
    let status = Resource::new_blocking(
        move || (),
        {
            let cid = cid.clone();
            let node = node1.clone();
            let gt = gt1.clone();
            move |_| {
                let cid = cid.clone();
                let node = node.clone();
                let gt = gt.clone();
                super::detail::get_guest_status(cid, node, vmid, gt)
            }
        },
    );

    // Fetch guest config
    let config = Resource::new_blocking(
        move || (),
        {
            let cid = cid2.clone();
            let node = node2.clone();
            let gt = gt2.clone();
            move |_| {
                let cid = cid.clone();
                let node = node.clone();
                let gt = gt.clone();
                super::detail::get_guest_config(cid, node, vmid, gt)
            }
        },
    );

    // Fetch RRD (hour)
    let rrd = Resource::new_blocking(
        move || (),
        {
            let cid = cid3.clone();
            let node = node3.clone();
            let gt = gt3.clone();
            move |_| {
                let cid = cid.clone();
                let node = node.clone();
                let gt = gt.clone();
                super::detail::get_guest_rrd(cid, node, vmid, gt, "hour".to_string())
            }
        },
    );

    view! {
        // Summary bar
        <Suspense fallback=|| view! { <div class="h-16 bg-surface-secondary rounded-lg animate-pulse mb-4"></div> }>
            {move || status.get().map(|result| match result {
                Ok(val) => {
                    let st = val.get("status").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                    let running = st == "running";
                    let uptime_val = val.get("uptime").and_then(|v| v.as_u64()).unwrap_or(0);
                    let cpu_val = val.get("cpu").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let mem_used = val.get("mem").and_then(|v| v.as_u64()).unwrap_or(0);
                    let mem_total = val.get("maxmem").and_then(|v| v.as_u64()).unwrap_or(0);
                    let disk_used = val.get("disk").and_then(|v| v.as_u64()).unwrap_or(0);
                    let disk_total = val.get("maxdisk").and_then(|v| v.as_u64()).unwrap_or(0);
                    let _name_val = val.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let node_str = node_display.clone();

                    let status_color = if running { Some("#4CAF50".to_string()) } else { Some("#EF4444".to_string()) };
                    let cpu_str = format!("{:.1}%", cpu_val * 100.0);
                    let mem_pct = if mem_total > 0 {
                        format!("{:.1}% ({}/{})", (mem_used as f64 / mem_total as f64) * 100.0, format_bytes(mem_used), format_bytes(mem_total))
                    } else {
                        "-".to_string()
                    };
                    let disk_str = if disk_total > 0 {
                        format!("{} / {}", format_bytes(disk_used), format_bytes(disk_total))
                    } else {
                        "-".to_string()
                    };

                    view! {
                        <SummaryBar items=vec![
                            SummaryItem {
                                label: "Status",
                                value: st,
                                color: status_color,
                                sparkline_data: None,
                                sparkline_color: None,
                            },
                            SummaryItem {
                                label: "Node",
                                value: node_str,
                                color: None,
                                sparkline_data: None,
                                sparkline_color: None,
                            },
                            SummaryItem {
                                label: "Uptime",
                                value: format_uptime(uptime_val),
                                color: None,
                                sparkline_data: None,
                                sparkline_color: None,
                            },
                            SummaryItem {
                                label: "CPU",
                                value: cpu_str,
                                color: None,
                                sparkline_data: None,
                                sparkline_color: None,
                            },
                            SummaryItem {
                                label: "Memory",
                                value: mem_pct,
                                color: None,
                                sparkline_data: None,
                                sparkline_color: None,
                            },
                            SummaryItem {
                                label: "Disk",
                                value: disk_str,
                                color: None,
                                sparkline_data: None,
                                sparkline_color: None,
                            },
                        ] />
                    }.into_any()
                }
                Err(e) => view! {
                    <p class="text-accent-danger text-sm mb-4">{format!("Status error: {}", e)}</p>
                }.into_any(),
            })}
        </Suspense>

        // Configuration section
        <Suspense fallback=|| view! { <div class="h-24 bg-surface-tertiary rounded-lg animate-pulse mb-4"></div> }>
            {move || config.get().map(|result: Result<AppGuestConfig, ServerFnError>| match result {
                Ok(cfg) => {
                    let cores = cfg.cores.map(|c: u32| c.to_string()).unwrap_or_else(|| "-".to_string());
                    let sockets = cfg.sockets.map(|s: u32| s.to_string()).unwrap_or_else(|| "1".to_string());
                    let memory_mb = cfg.memory.unwrap_or(0);
                    let memory_str = if memory_mb > 0 { format!("{} MiB", memory_mb) } else { "-".to_string() };
                    let ostype = cfg.ostype.clone().unwrap_or_else(|| "-".to_string());
                    let disk_devs = cfg.disk_devices.clone();
                    let net_devs = cfg.net_devices.clone();

                    view! {
                        <div class="bg-surface-tertiary border border-border-primary rounded-lg p-4 mb-4">
                            <h3 class="text-sm font-medium text-text-secondary mb-3">"Configuration"</h3>
                            <div class="grid grid-cols-2 md:grid-cols-4 gap-3 text-[12px]">
                                <div>
                                    <span class="text-text-muted block">"Cores"</span>
                                    <span class="text-text-primary font-medium">{cores}</span>
                                </div>
                                <div>
                                    <span class="text-text-muted block">"Sockets"</span>
                                    <span class="text-text-primary font-medium">{sockets}</span>
                                </div>
                                <div>
                                    <span class="text-text-muted block">"Memory"</span>
                                    <span class="text-text-primary font-medium">{memory_str}</span>
                                </div>
                                <div>
                                    <span class="text-text-muted block">"OS Type"</span>
                                    <span class="text-text-primary font-medium">{ostype}</span>
                                </div>
                            </div>
                            // Disk devices
                            {if !disk_devs.is_empty() {
                                view! {
                                    <div class="mt-3">
                                        <span class="text-text-muted text-[11px] block mb-1">"Disk Devices"</span>
                                        {disk_devs.into_iter().map(|d| view! {
                                            <div class="text-text-secondary text-[11px] font-mono truncate">{d}</div>
                                        }).collect_view()}
                                    </div>
                                }.into_any()
                            } else {
                                view! {}.into_any()
                            }}
                            // Network devices
                            {if !net_devs.is_empty() {
                                view! {
                                    <div class="mt-3">
                                        <span class="text-text-muted text-[11px] block mb-1">"Network Devices"</span>
                                        {net_devs.into_iter().map(|n| view! {
                                            <div class="text-text-secondary text-[11px] font-mono truncate">{n}</div>
                                        }).collect_view()}
                                    </div>
                                }.into_any()
                            } else {
                                view! {}.into_any()
                            }}
                        </div>
                    }.into_any()
                }
                Err(e) => view! {
                    <p class="text-accent-danger text-sm mb-4">{format!("Config error: {}", e)}</p>
                }.into_any(),
            })}
        </Suspense>

        // RRD chart panels (2x2 grid)
        <Suspense fallback=|| view! { <div class="grid grid-cols-2 gap-4"><div class="h-28 bg-surface-tertiary rounded-lg animate-pulse"></div><div class="h-28 bg-surface-tertiary rounded-lg animate-pulse"></div><div class="h-28 bg-surface-tertiary rounded-lg animate-pulse"></div><div class="h-28 bg-surface-tertiary rounded-lg animate-pulse"></div></div> }>
            {move || rrd.get().map(|result: Result<Vec<RrdPoint>, ServerFnError>| match result {
                Ok(points) => {
                    let cpu: Vec<f64> = points.iter().filter_map(|p| p.cpu).map(|v| v * 100.0).collect();
                    let mem: Vec<f64> = points.iter().filter_map(|p| p.mem).collect();
                    let netin: Vec<f64> = points.iter().filter_map(|p| p.netin).collect();
                    let netout: Vec<f64> = points.iter().filter_map(|p| p.netout).collect();
                    let diskread: Vec<f64> = points.iter().filter_map(|p| p.diskread).collect();
                    let diskwrite: Vec<f64> = points.iter().filter_map(|p| p.diskwrite).collect();

                    view! {
                        <div class="grid grid-cols-2 gap-4">
                            <div class="bg-surface-tertiary border border-border-primary rounded-lg p-3">
                                <div class="text-text-muted text-[11px] mb-2">"CPU Usage (1 hour)"</div>
                                <Sparkline data=cpu color="#F59E0B".to_string() width=400 height=80 />
                            </div>
                            <div class="bg-surface-tertiary border border-border-primary rounded-lg p-3">
                                <div class="text-text-muted text-[11px] mb-2">"Memory (1 hour)"</div>
                                <Sparkline data=mem color="#A78BFA".to_string() width=400 height=80 />
                            </div>
                            <div class="bg-surface-tertiary border border-border-primary rounded-lg p-3">
                                <div class="text-text-muted text-[11px] mb-2">"Network I/O (1 hour)"</div>
                                <Sparkline data=netin color="#4CAF50".to_string() width=400 height=40 />
                                <Sparkline data=netout color="#F44336".to_string() width=400 height=40 fill=false />
                                <div class="flex gap-4 mt-1 text-[10px] text-text-muted">
                                    <span class="flex items-center gap-1">
                                        <span class="w-2 h-0.5 bg-[#4CAF50] inline-block"></span> "In"
                                    </span>
                                    <span class="flex items-center gap-1">
                                        <span class="w-2 h-0.5 bg-[#F44336] inline-block"></span> "Out"
                                    </span>
                                </div>
                            </div>
                            <div class="bg-surface-tertiary border border-border-primary rounded-lg p-3">
                                <div class="text-text-muted text-[11px] mb-2">"Disk I/O (1 hour)"</div>
                                <Sparkline data=diskread color="#4CAF50".to_string() width=400 height=40 />
                                <Sparkline data=diskwrite color="#F44336".to_string() width=400 height=40 fill=false />
                                <div class="flex gap-4 mt-1 text-[10px] text-text-muted">
                                    <span class="flex items-center gap-1">
                                        <span class="w-2 h-0.5 bg-[#4CAF50] inline-block"></span> "Read"
                                    </span>
                                    <span class="flex items-center gap-1">
                                        <span class="w-2 h-0.5 bg-[#F44336] inline-block"></span> "Write"
                                    </span>
                                </div>
                            </div>
                        </div>
                    }.into_any()
                }
                Err(e) => view! {
                    <p class="text-accent-danger text-sm">{format!("RRD error: {}", e)}</p>
                }.into_any(),
            })}
        </Suspense>
    }
}
