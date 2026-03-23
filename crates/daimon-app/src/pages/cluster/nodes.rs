use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use crate::components::table::MetricTable;
use super::detail::get_cluster_nodes;

#[component]
pub fn Nodes() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();

    let nodes = Resource::new(
        move || cluster_id(),
        |cid| get_cluster_nodes(cid),
    );

    view! {
        <Suspense fallback=|| view! { <div class="text-text-muted text-sm">"Loading nodes..."</div> }>
            {move || nodes.get().map(|result| match result {
                Ok(rows) => view! { <MetricTable rows=rows /> }.into_any(),
                Err(e) => view! {
                    <div class="text-accent-danger text-sm">{format!("Error: {}", e)}</div>
                }.into_any(),
            })}
        </Suspense>
    }
}
