use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use leptos_router::components::Outlet;
use crate::components::detail_layout::{DetailLayout, DetailTab};

// Reuse the shared GuestOverviewInner from vm_detail
use super::vm_detail::GuestOverviewInner;

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

#[component]
pub fn LxcOverview() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let vmid_str = move || params.get().get("vmid").unwrap_or_default();
    let vmid = move || vmid_str().parse::<u32>().unwrap_or(0);

    // Find which node this LXC is on
    let node_info = Resource::new(
        move || (cluster_id(), vmid()),
        |(cid, vid)| super::detail::find_guest_node(cid, vid),
    );

    view! {
        <Suspense fallback=|| view! { <p class="text-text-muted text-sm">"Locating container..."</p> }>
            {move || node_info.get().map(|result| match result {
                Ok((node_name, _guest_type)) => {
                    let cid = cluster_id();
                    let vid = vmid();
                    let node = node_name.clone();
                    view! {
                        <GuestOverviewInner
                            cluster_id=cid
                            node=node
                            vmid=vid
                            guest_type="lxc".to_string()
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
