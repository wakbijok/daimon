use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use leptos_router::components::Outlet;
use crate::components::detail_layout::{DetailLayout, DetailTab};
use crate::components::summary_bar::{SummaryBar, SummaryItem};
use crate::components::sparkline::Sparkline;
use crate::components::table::format_bytes;

#[component]
pub fn StorageDetail() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let node_name = move || params.get().get("node_name").unwrap_or_default();
    let storage_name = move || params.get().get("storage_name").unwrap_or_default();
    let base = move || format!(
        "/clusters/{}/nodes/{}/storage/{}",
        cluster_id(), node_name(), storage_name()
    );

    view! {
        <DetailLayout
            title=storage_name()
            subtitle=format!("Storage on {}", node_name())
            tabs=vec![
                DetailTab { label: "Overview", path: base(), requires_agent: false },
                DetailTab { label: "Devices", path: format!("{}/devices", base()), requires_agent: true },
                DetailTab { label: "Usage", path: format!("{}/usage", base()), requires_agent: false },
                DetailTab { label: "Charts", path: format!("{}/charts", base()), requires_agent: false },
            ]
        >
            <Outlet />
        </DetailLayout>
    }
}

#[component]
pub fn StorageOverview() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let node_name = move || params.get().get("node_name").unwrap_or_default();
    let storage_name = move || params.get().get("storage_name").unwrap_or_default();

    // Fetch storage status
    let status = Resource::new(
        move || (cluster_id(), node_name(), storage_name()),
        |(cid, node, storage)| super::detail::get_storage_status(cid, node, storage),
    );

    // Fetch storage RRD (hour)
    let rrd = Resource::new(
        move || (cluster_id(), node_name(), storage_name()),
        |(cid, node, storage)| super::detail::get_storage_rrd(cid, node, storage, "hour".to_string()),
    );

    view! {
        // Summary bar
        <Suspense fallback=|| view! { <div class="h-16 bg-surface-secondary rounded-lg animate-pulse mb-4"></div> }>
            {move || status.get().map(|result| match result {
                Ok(s) => {
                    let usage_pct = if s.total > 0 {
                        format!("{:.1}%", (s.used as f64 / s.total as f64) * 100.0)
                    } else {
                        "-".to_string()
                    };
                    let used_str = format_bytes(s.used);
                    let total_str = format_bytes(s.total);
                    let avail_str = format_bytes(s.avail);
                    let active_str = if s.active { "Active" } else { "Inactive" };
                    let active_color = if s.active { Some("#4CAF50".to_string()) } else { Some("#EF4444".to_string()) };
                    let shared_str = if s.shared { "Yes" } else { "No" };
                    let stype = s.storage_type.clone();
                    let content = s.content.clone();

                    view! {
                        <SummaryBar items=vec![
                            SummaryItem {
                                label: "Status",
                                value: active_str.to_string(),
                                color: active_color,
                                sparkline_data: None,
                                sparkline_color: None,
                            },
                            SummaryItem {
                                label: "Type",
                                value: stype,
                                color: None,
                                sparkline_data: None,
                                sparkline_color: None,
                            },
                            SummaryItem {
                                label: "Usage",
                                value: usage_pct,
                                color: None,
                                sparkline_data: None,
                                sparkline_color: None,
                            },
                            SummaryItem {
                                label: "Used",
                                value: used_str,
                                color: None,
                                sparkline_data: None,
                                sparkline_color: None,
                            },
                            SummaryItem {
                                label: "Total",
                                value: total_str,
                                color: None,
                                sparkline_data: None,
                                sparkline_color: None,
                            },
                            SummaryItem {
                                label: "Available",
                                value: avail_str,
                                color: None,
                                sparkline_data: None,
                                sparkline_color: None,
                            },
                            SummaryItem {
                                label: "Shared",
                                value: shared_str.to_string(),
                                color: None,
                                sparkline_data: None,
                                sparkline_color: None,
                            },
                            SummaryItem {
                                label: "Content",
                                value: content,
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

        // Storage usage chart
        <Suspense fallback=|| view! { <div class="h-28 bg-surface-tertiary rounded-lg animate-pulse"></div> }>
            {move || rrd.get().map(|result| match result {
                Ok(points) => {
                    let used: Vec<f64> = points.iter().filter_map(|p| p.disk).collect();
                    let total: Vec<f64> = points.iter().filter_map(|p| p.maxdisk).collect();

                    view! {
                        <div class="bg-surface-tertiary border border-border-primary rounded-lg p-3">
                            <div class="text-text-muted text-[11px] mb-2">"Storage Usage (1 hour)"</div>
                            <Sparkline data=used color="#A78BFA".to_string() width=600 height=100 />
                            {if !total.is_empty() {
                                view! {
                                    <Sparkline data=total color="#6B7280".to_string() width=600 height=100 fill=false />
                                }.into_any()
                            } else {
                                view! {}.into_any()
                            }}
                            <div class="flex gap-4 mt-1 text-[10px] text-text-muted">
                                <span class="flex items-center gap-1">
                                    <span class="w-2 h-0.5 bg-[#A78BFA] inline-block"></span> "Used"
                                </span>
                                <span class="flex items-center gap-1">
                                    <span class="w-2 h-0.5 bg-[#6B7280] inline-block"></span> "Total"
                                </span>
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
