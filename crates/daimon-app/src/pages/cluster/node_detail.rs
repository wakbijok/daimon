use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use leptos_router::components::Outlet;
use crate::components::detail_layout::{DetailLayout, DetailTab};
use crate::components::summary_bar::{SummaryBar, SummaryItem};
use crate::components::sparkline::Sparkline;
use crate::components::table::{format_bytes, format_uptime};

#[component]
pub fn NodeDetail() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let node_name = move || params.get().get("node_name").unwrap_or_default();
    let base = move || format!("/clusters/{}/nodes/{}", cluster_id(), node_name());

    view! {
        <DetailLayout
            title=node_name()
            subtitle="PVE Node".to_string()
            tabs=vec![
                DetailTab { label: "Overview", path: base(), requires_agent: false },
                DetailTab { label: "Hardware", path: format!("{}/hardware", base()), requires_agent: true },
                DetailTab { label: "RAID", path: format!("{}/raid", base()), requires_agent: true },
                DetailTab { label: "Disks", path: format!("{}/disks", base()), requires_agent: true },
                DetailTab { label: "Storage", path: format!("{}/storage", base()), requires_agent: true },
                DetailTab { label: "Network", path: format!("{}/network", base()), requires_agent: true },
                DetailTab { label: "Services", path: format!("{}/services", base()), requires_agent: true },
                DetailTab { label: "Charts", path: format!("{}/charts", base()), requires_agent: false },
            ]
        >
            <Outlet />
        </DetailLayout>
    }
}

#[component]
pub fn NodeOverview() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let node_name = move || params.get().get("node_name").unwrap_or_default();

    // Fetch node status
    let status = Resource::new(
        move || (cluster_id(), node_name()),
        |(cid, node)| super::detail::get_node_status(cid, node),
    );

    // Fetch RRD data (hour timeframe for overview sparklines)
    let rrd = Resource::new(
        move || (cluster_id(), node_name()),
        |(cid, node)| super::detail::get_node_rrd(cid, node, "hour".to_string()),
    );

    // Fetch guest list on this node
    let guests = Resource::new(
        move || (cluster_id(), node_name()),
        |(cid, node)| super::detail::get_node_guests(cid, node),
    );

    view! {
        // Summary bar from node status
        <Suspense fallback=|| view! { <div class="h-16 bg-surface-secondary rounded-lg animate-pulse mb-4"></div> }>
            {move || {
                let rrd_data = rrd.get().and_then(|r| r.ok());
                let cpu_spark: Vec<f64> = rrd_data.as_ref()
                    .map(|pts| pts.iter().filter_map(|p| p.cpu).map(|v| v * 100.0).collect())
                    .unwrap_or_default();
                let mem_spark: Vec<f64> = rrd_data.as_ref()
                    .map(|pts| pts.iter().filter_map(|p| p.mem).collect())
                    .unwrap_or_default();

                status.get().map(|result| match result {
                    Ok(s) => {
                        let cpu_pct = format!("{:.1}%", s.cpu * 100.0);
                        let mem_pct = s.memory.as_ref().map(|m| {
                            if m.total > 0 { format!("{:.1}%", (m.used as f64 / m.total as f64) * 100.0) }
                            else { "0%".to_string() }
                        }).unwrap_or_else(|| "-".to_string());
                        let uptime_str = format_uptime(s.uptime);
                        let pve_ver = s.pveversion.clone().unwrap_or_else(|| "-".to_string());
                        let kernel = s.kversion.clone().unwrap_or_else(|| "-".to_string());
                        let kernel_short = if kernel.len() > 30 {
                            format!("{}...", &kernel[..30])
                        } else {
                            kernel
                        };

                        view! {
                            <SummaryBar items=vec![
                                SummaryItem {
                                    label: "Status",
                                    value: "Online".to_string(),
                                    color: Some("#4CAF50".to_string()),
                                    sparkline_data: None,
                                    sparkline_color: None,
                                },
                                SummaryItem {
                                    label: "Uptime",
                                    value: uptime_str,
                                    color: None,
                                    sparkline_data: None,
                                    sparkline_color: None,
                                },
                                SummaryItem {
                                    label: "CPU",
                                    value: cpu_pct,
                                    color: None,
                                    sparkline_data: Some(cpu_spark.clone()),
                                    sparkline_color: Some("#F59E0B".to_string()),
                                },
                                SummaryItem {
                                    label: "Memory",
                                    value: mem_pct,
                                    color: None,
                                    sparkline_data: Some(mem_spark.clone()),
                                    sparkline_color: Some("#A78BFA".to_string()),
                                },
                                SummaryItem {
                                    label: "PVE",
                                    value: pve_ver,
                                    color: None,
                                    sparkline_data: None,
                                    sparkline_color: None,
                                },
                                SummaryItem {
                                    label: "Kernel",
                                    value: kernel_short,
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
                })
            }}
        </Suspense>

        // RRD chart panels (2x2 grid)
        <Suspense fallback=|| view! { <div class="grid grid-cols-2 gap-4 mb-4"><div class="h-28 bg-surface-tertiary rounded-lg animate-pulse"></div><div class="h-28 bg-surface-tertiary rounded-lg animate-pulse"></div><div class="h-28 bg-surface-tertiary rounded-lg animate-pulse"></div><div class="h-28 bg-surface-tertiary rounded-lg animate-pulse"></div></div> }>
            {move || rrd.get().map(|result| match result {
                Ok(points) => {
                    let cpu: Vec<f64> = points.iter().filter_map(|p| p.cpu).map(|v| v * 100.0).collect();
                    let mem: Vec<f64> = points.iter().filter_map(|p| p.mem).collect();
                    let netin: Vec<f64> = points.iter().filter_map(|p| p.netin).collect();
                    let netout: Vec<f64> = points.iter().filter_map(|p| p.netout).collect();
                    let diskread: Vec<f64> = points.iter().filter_map(|p| p.diskread).collect();
                    let diskwrite: Vec<f64> = points.iter().filter_map(|p| p.diskwrite).collect();

                    view! {
                        <div class="grid grid-cols-2 gap-4 mb-4">
                            <div class="bg-surface-tertiary border border-border-primary rounded-lg p-3">
                                <div class="text-text-muted text-[11px] mb-2">"CPU Usage (1 hour)"</div>
                                <Sparkline data=cpu color="#F59E0B".to_string() width=400 height=80 />
                            </div>
                            <div class="bg-surface-tertiary border border-border-primary rounded-lg p-3">
                                <div class="text-text-muted text-[11px] mb-2">"Memory Usage (1 hour)"</div>
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
                    <p class="text-accent-danger text-sm mb-4">{format!("RRD error: {}", e)}</p>
                }.into_any(),
            })}
        </Suspense>

        // Guest list
        <div class="mb-2">
            <h3 class="text-sm font-medium text-text-secondary mb-2">"Guests on this node"</h3>
        </div>
        <Suspense fallback=|| view! { <p class="text-text-muted text-sm">"Loading guests..."</p> }>
            {move || {
                let cid = cluster_id();
                guests.get().map(|result| match result {
                    Ok(guest_list) => {
                        if guest_list.is_empty() {
                            return view! {
                                <p class="text-text-muted text-sm">"No guests found on this node."</p>
                            }.into_any();
                        }
                        view! {
                            <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2">
                                {guest_list.into_iter().map(|g| {
                                    let running = g.status == "running";
                                    let vmid = g.vmid;
                                    let name = g.name.clone();
                                    let _status = g.status.clone();
                                    let guest_type = g.guest_type.clone();
                                    let type_label = if guest_type == "qemu" { "VM" } else { "CT" };
                                    let cpu_str = format!("{:.1}%", g.cpu_pct);
                                    let mem_str = format!("{} / {}", format_bytes(g.mem_used), format_bytes(g.mem_total));
                                    let uptime_str = format_uptime(g.uptime);
                                    let route_segment = if guest_type == "qemu" { "vms" } else { "containers" };
                                    let href = format!("/clusters/{}/{}/{}", cid, route_segment, vmid);
                                    view! {
                                        <a
                                            href=href
                                            class="block bg-surface-primary border border-border-primary p-3 rounded-lg cursor-pointer hover:bg-surface-tertiary transition-colors no-underline"
                                        >
                                            <div class="flex justify-between items-center mb-1">
                                                <span class="text-text-primary text-sm font-medium flex items-center gap-1.5">
                                                    <span class=format!("w-2 h-2 rounded-full {}",
                                                        if running { "bg-accent-green" } else { "bg-accent-danger" })>
                                                    </span>
                                                    {format!("{} {}", vmid, name)}
                                                </span>
                                                <span class="text-text-muted text-[11px]">{type_label}</span>
                                            </div>
                                            <div class="flex gap-4 text-[11px] text-text-muted">
                                                <span>{format!("CPU: {}", cpu_str)}</span>
                                                <span>{format!("Mem: {}", mem_str)}</span>
                                                <span>{format!("Up: {}", uptime_str)}</span>
                                            </div>
                                        </a>
                                    }
                                }).collect_view()}
                            </div>
                        }.into_any()
                    }
                    Err(e) => view! {
                        <p class="text-accent-danger text-sm">{format!("Guest list error: {}", e)}</p>
                    }.into_any(),
                })
            }}
        </Suspense>
    }
}
