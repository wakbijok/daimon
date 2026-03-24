use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use leptos_router::components::Outlet;
use crate::components::detail_layout::{DetailLayout, DetailTab};

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

/// Default overview tab content — placeholder for now, will be populated in Task 11
#[component]
pub fn NodeOverview() -> impl IntoView {
    view! { <p class="text-text-muted text-sm">"Node overview — loading PVE data..."</p> }
}
