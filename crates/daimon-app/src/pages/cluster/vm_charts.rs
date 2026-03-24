use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use crate::components::sparkline::Sparkline;

#[component]
pub fn VmCharts() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let vmid = move || params.get().get("vmid").unwrap_or_default().parse::<u32>().unwrap_or(0);

    // Find node first
    let node_info = Resource::new(
        move || (cluster_id(), vmid()),
        |(cid, vid)| super::detail::find_guest_node(cid, vid),
    );

    view! {
        <Suspense fallback=|| view! { <p class="text-text-muted text-sm">"Locating VM..."</p> }>
            {move || node_info.get().map(|result| match result {
                Ok((node_name, _gt)) => {
                    let cid = cluster_id();
                    let vid = vmid();
                    view! {
                        <GuestChartsInner
                            cluster_id=cid
                            node=node_name
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

/// Shared chart component for VM and LXC chart pages
#[component]
pub fn GuestChartsInner(
    cluster_id: String,
    node: String,
    vmid: u32,
    guest_type: String,
) -> impl IntoView {
    let (timeframe, set_timeframe) = signal("hour".to_string());

    let cid = cluster_id.clone();
    let node1 = node.clone();
    let gt1 = guest_type.clone();

    let rrd = Resource::new(
        move || timeframe.get(),
        {
            let cid = cid.clone();
            let node = node1.clone();
            let gt = gt1.clone();
            move |tf| {
                let cid = cid.clone();
                let node = node.clone();
                let gt = gt.clone();
                super::detail::get_guest_rrd(cid, node, vmid, gt, tf)
            }
        },
    );

    view! {
        // Timeframe selector
        <div class="flex gap-1 mb-4">
            {["hour", "day", "week", "month", "year"].into_iter().map(|tf| {
                let tf_str = tf.to_string();
                let tf_str2 = tf_str.clone();
                let tf_label = match tf {
                    "hour" => "Hour",
                    "day" => "Day",
                    "week" => "Week",
                    "month" => "Month",
                    "year" => "Year",
                    _ => tf,
                };
                view! {
                    <button
                        on:click=move |_| set_timeframe.set(tf_str.clone())
                        class=move || format!("px-3 py-1.5 text-xs rounded-md border transition-colors {}",
                            if timeframe.get() == tf_str2 {
                                "bg-accent-amber text-surface-primary border-accent-amber"
                            } else {
                                "text-text-muted border-border-primary hover:text-text-secondary"
                            })
                    >
                        {tf_label}
                    </button>
                }
            }).collect_view()}
        </div>

        // Chart panels
        <Suspense fallback=|| view! { <p class="text-text-muted text-sm">"Loading charts..."</p> }>
            {move || rrd.get().map(|result| match result {
                Ok(points) => {
                    let cpu: Vec<f64> = points.iter().filter_map(|p| p.cpu).map(|v| v * 100.0).collect();
                    let mem: Vec<f64> = points.iter().filter_map(|p| p.mem).collect();
                    let netin: Vec<f64> = points.iter().filter_map(|p| p.netin).collect();
                    let netout: Vec<f64> = points.iter().filter_map(|p| p.netout).collect();
                    let diskread: Vec<f64> = points.iter().filter_map(|p| p.diskread).collect();
                    let diskwrite: Vec<f64> = points.iter().filter_map(|p| p.diskwrite).collect();
                    let tf = timeframe.get();

                    view! {
                        <div class="grid grid-cols-2 gap-4">
                            <div class="bg-surface-tertiary border border-border-primary rounded-lg p-4">
                                <div class="text-text-muted text-[11px] mb-2">{format!("CPU Usage ({})", tf)}</div>
                                <Sparkline data=cpu color="#F59E0B".to_string() width=500 height=120 />
                            </div>
                            <div class="bg-surface-tertiary border border-border-primary rounded-lg p-4">
                                <div class="text-text-muted text-[11px] mb-2">{format!("Memory ({})", tf)}</div>
                                <Sparkline data=mem color="#A78BFA".to_string() width=500 height=120 />
                            </div>
                            <div class="bg-surface-tertiary border border-border-primary rounded-lg p-4">
                                <div class="text-text-muted text-[11px] mb-2">{format!("Network I/O ({})", tf)}</div>
                                <Sparkline data=netin color="#4CAF50".to_string() width=500 height=55 />
                                <Sparkline data=netout color="#F44336".to_string() width=500 height=55 fill=false />
                                <div class="flex gap-4 mt-1 text-[10px] text-text-muted">
                                    <span class="flex items-center gap-1">
                                        <span class="w-2 h-0.5 bg-[#4CAF50] inline-block"></span> "In"
                                    </span>
                                    <span class="flex items-center gap-1">
                                        <span class="w-2 h-0.5 bg-[#F44336] inline-block"></span> "Out"
                                    </span>
                                </div>
                            </div>
                            <div class="bg-surface-tertiary border border-border-primary rounded-lg p-4">
                                <div class="text-text-muted text-[11px] mb-2">{format!("Disk I/O ({})", tf)}</div>
                                <Sparkline data=diskread color="#4CAF50".to_string() width=500 height=55 />
                                <Sparkline data=diskwrite color="#F44336".to_string() width=500 height=55 fill=false />
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
                Err(e) => view! { <p class="text-accent-danger text-sm">{e.to_string()}</p> }.into_any(),
            })}
        </Suspense>
    }
}
