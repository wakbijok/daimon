use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use leptos_router::components::Outlet;
use crate::components::detail_layout::{DetailLayout, DetailTab};

#[component]
pub fn LxcDetail() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let vmid = move || params.get().get("vmid").unwrap_or_default();
    let base = move || format!("/clusters/{}/containers/{}", cluster_id(), vmid());

    view! {
        <DetailLayout
            title=format!("CT {}", vmid())
            subtitle="LXC Container".to_string()
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

/// Default overview tab content — placeholder for now, will be populated in Task 11
#[component]
pub fn LxcOverview() -> impl IntoView {
    view! { <p class="text-text-muted text-sm">"LXC overview — loading PVE data..."</p> }
}
