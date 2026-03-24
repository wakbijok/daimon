use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

// Reuse the shared GuestChartsInner from vm_charts
use super::vm_charts::GuestChartsInner;

#[component]
pub fn LxcCharts() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let vmid = move || params.get().get("vmid").unwrap_or_default().parse::<u32>().unwrap_or(0);

    // Find node first
    let node_info = Resource::new(
        move || (cluster_id(), vmid()),
        |(cid, vid)| super::detail::find_guest_node(cid, vid),
    );

    view! {
        <Suspense fallback=|| view! { <p class="text-text-muted text-sm">"Locating container..."</p> }>
            {move || node_info.get().map(|result| match result {
                Ok((node_name, _gt)) => {
                    let cid = cluster_id();
                    let vid = vmid();
                    view! {
                        <GuestChartsInner
                            cluster_id=cid
                            node=node_name
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
