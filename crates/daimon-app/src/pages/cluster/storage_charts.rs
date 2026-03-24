use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use crate::components::sparkline::Sparkline;
use super::detail::RrdPoint;

#[component]
pub fn StorageCharts() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let node_name = move || params.get().get("node_name").unwrap_or_default();
    let storage_name = move || params.get().get("storage_name").unwrap_or_default();
    let (timeframe, set_timeframe) = signal("hour".to_string());

    let rrd = Resource::new(
        move || (cluster_id(), node_name(), storage_name(), timeframe.get()),
        |(cid, node, storage, tf)| super::detail::get_storage_rrd(cid, node, storage, tf),
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

        // Chart panel — storage RRD has disk/maxdisk
        <Suspense fallback=|| view! { <p class="text-text-muted text-sm">"Loading charts..."</p> }>
            {move || rrd.get().map(|result: Result<Vec<RrdPoint>, ServerFnError>| match result {
                Ok(points) => {
                    let used: Vec<f64> = points.iter().filter_map(|p| p.disk).collect();
                    let total: Vec<f64> = points.iter().filter_map(|p| p.maxdisk).collect();
                    let tf = timeframe.get();

                    view! {
                        <div class="grid grid-cols-1 gap-4">
                            <div class="bg-surface-tertiary border border-border-primary rounded-lg p-4">
                                <div class="text-text-muted text-[11px] mb-2">{format!("Storage Usage ({})", tf)}</div>
                                <Sparkline data=used color="#A78BFA".to_string() width=800 height=150 />
                                {if !total.is_empty() {
                                    view! {
                                        <Sparkline data=total color="#6B7280".to_string() width=800 height=150 fill=false />
                                    }.into_any()
                                } else {
                                    view! {}.into_any()
                                }}
                                <div class="flex gap-4 mt-2 text-[10px] text-text-muted">
                                    <span class="flex items-center gap-1">
                                        <span class="w-2 h-0.5 bg-[#A78BFA] inline-block"></span> "Used"
                                    </span>
                                    <span class="flex items-center gap-1">
                                        <span class="w-2 h-0.5 bg-[#6B7280] inline-block"></span> "Total"
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
