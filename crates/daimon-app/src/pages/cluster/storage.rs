use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use crate::components::table::StorageTable;
use super::detail::get_cluster_storage;

#[component]
pub fn Storage() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let data = Resource::new(move || cluster_id(), |cid| get_cluster_storage(cid));

    view! {
        <Suspense fallback=|| view! { <p class="text-text-muted text-sm">"Loading storage..."</p> }>
            {move || data.get().map(|result| match result {
                Ok(rows) if rows.is_empty() => view! { <p class="text-text-muted text-sm">"No storage found"</p> }.into_any(),
                Ok(rows) => view! { <StorageTable rows=rows /> }.into_any(),
                Err(e) => view! { <p class="text-accent-danger text-sm">{e.to_string()}</p> }.into_any(),
            })}
        </Suspense>
    }
}
