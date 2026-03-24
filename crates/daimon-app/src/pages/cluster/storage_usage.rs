use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use crate::components::table::format_bytes;

#[component]
pub fn StorageUsage() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let node_name = move || params.get().get("node_name").unwrap_or_default();
    let storage_name = move || params.get().get("storage_name").unwrap_or_default();

    // Fetch storage status for summary info
    let status = Resource::new(
        move || (cluster_id(), node_name(), storage_name()),
        |(cid, node, storage)| super::detail::get_storage_status(cid, node, storage),
    );

    view! {
        <Suspense fallback=|| view! { <p class="text-text-muted text-sm">"Loading storage usage..."</p> }>
            {move || status.get().map(|result| match result {
                Ok(s) => {
                    let used_pct = if s.total > 0 {
                        (s.used as f64 / s.total as f64) * 100.0
                    } else {
                        0.0
                    };
                    let bar_width = format!("width: {}%", used_pct.min(100.0));
                    let bar_color = if used_pct > 90.0 {
                        "bg-accent-danger"
                    } else if used_pct > 75.0 {
                        "bg-accent-amber"
                    } else {
                        "bg-accent-green"
                    };

                    view! {
                        <div class="space-y-4">
                            // Usage bar
                            <div class="bg-surface-tertiary border border-border-primary rounded-lg p-4">
                                <h3 class="text-sm font-medium text-text-secondary mb-3">"Capacity Overview"</h3>
                                <div class="flex items-center gap-4 mb-2">
                                    <div class="flex-1 h-4 bg-surface-primary rounded-full overflow-hidden">
                                        <div class=format!("h-full rounded-full transition-all {}", bar_color)
                                            style=bar_width>
                                        </div>
                                    </div>
                                    <span class="text-text-primary text-sm font-mono">
                                        {format!("{:.1}%", used_pct)}
                                    </span>
                                </div>
                                <div class="grid grid-cols-3 gap-4 text-[12px] mt-3">
                                    <div>
                                        <span class="text-text-muted block">"Used"</span>
                                        <span class="text-text-primary font-medium">{format_bytes(s.used)}</span>
                                    </div>
                                    <div>
                                        <span class="text-text-muted block">"Available"</span>
                                        <span class="text-text-primary font-medium">{format_bytes(s.avail)}</span>
                                    </div>
                                    <div>
                                        <span class="text-text-muted block">"Total"</span>
                                        <span class="text-text-primary font-medium">{format_bytes(s.total)}</span>
                                    </div>
                                </div>
                            </div>

                            // Storage details
                            <div class="bg-surface-tertiary border border-border-primary rounded-lg p-4">
                                <h3 class="text-sm font-medium text-text-secondary mb-3">"Storage Details"</h3>
                                <div class="grid grid-cols-2 md:grid-cols-4 gap-3 text-[12px]">
                                    <div>
                                        <span class="text-text-muted block">"Type"</span>
                                        <span class="text-text-primary font-medium">{s.storage_type.clone()}</span>
                                    </div>
                                    <div>
                                        <span class="text-text-muted block">"Content Types"</span>
                                        <span class="text-text-primary font-medium">{s.content.clone()}</span>
                                    </div>
                                    <div>
                                        <span class="text-text-muted block">"Shared"</span>
                                        <span class="text-text-primary font-medium">
                                            {if s.shared { "Yes" } else { "No" }}
                                        </span>
                                    </div>
                                    <div>
                                        <span class="text-text-muted block">"Status"</span>
                                        <span class=format!("font-medium {}",
                                            if s.active { "text-accent-green" } else { "text-accent-danger" })>
                                            {if s.active { "Active" } else { "Inactive" }}
                                        </span>
                                    </div>
                                </div>
                            </div>

                            // Placeholder for VM config parsing
                            <div class="bg-surface-tertiary border border-border-primary rounded-lg p-4">
                                <h3 class="text-sm font-medium text-text-secondary mb-2">"Guest Usage Breakdown"</h3>
                                <p class="text-text-muted text-[12px]">
                                    "Per-guest storage allocation requires VM config parsing."
                                    <br />
                                    "This will be implemented when the agent layer can gather per-disk allocation data."
                                </p>
                            </div>
                        </div>
                    }.into_any()
                }
                Err(e) => view! {
                    <p class="text-accent-danger text-sm">{format!("Error: {}", e)}</p>
                }.into_any(),
            })}
        </Suspense>
    }
}
