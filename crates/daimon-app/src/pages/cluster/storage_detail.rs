use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use leptos_router::components::Outlet;
use crate::components::detail_layout::{DetailLayout, DetailTab};

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

/// Default overview tab content — placeholder for now, will be populated in Task 11
#[component]
pub fn StorageOverview() -> impl IntoView {
    view! { <p class="text-text-muted text-sm">"Storage overview — loading PVE data..."</p> }
}
