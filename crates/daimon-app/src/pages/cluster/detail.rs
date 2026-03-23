use leptos::prelude::*;
use leptos_router::components::Outlet;
use leptos_router::hooks::use_params_map;
use crate::components::tabs::{Tab, TabBar};
use crate::components::table::{NodeRow, GuestRow, StorageRow};

#[server]
pub async fn get_cluster_info(cluster_id: String) -> Result<(String, String), ServerFnError> {
    use crate::state::AppState;
    use crate::db;

    let state = expect_context::<AppState>();
    let conn = state.db.lock().await;
    let (_id, name, api_url, _token, _notes, _created) = db::get_cluster(&conn, &cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster not found"))?;
    Ok((name, api_url))
}

#[server]
pub async fn get_cluster_nodes(cluster_id: String) -> Result<Vec<NodeRow>, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster client not found"))?;

    let resources = client.cluster_resources(Some("node")).await
        .map_err(|e| ServerFnError::new(format!("PVE API error: {}", e)))?;

    Ok(resources.iter().map(|r| NodeRow {
        name: r.node.clone(),
        status: r.status.clone(),
        cpu_pct: r.cpu * 100.0,
        cpu_count: r.maxcpu,
        mem_used: r.mem,
        mem_total: r.maxmem,
        disk_used: r.disk,
        disk_total: r.maxdisk,
        uptime: r.uptime,
    }).collect())
}

#[server]
pub async fn get_cluster_vms(cluster_id: String) -> Result<Vec<GuestRow>, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster client not found"))?;

    let resources = client.cluster_resources(Some("vm")).await
        .map_err(|e| ServerFnError::new(format!("PVE API error: {}", e)))?;

    Ok(resources.iter().filter_map(|r| {
        r.vmid.map(|vmid| GuestRow {
            vmid,
            name: r.name.clone(),
            node: r.node.clone(),
            status: r.status.clone(),
            cpu_pct: r.cpu * 100.0,
            cpu_count: r.maxcpu,
            mem_used: r.mem,
            mem_total: r.maxmem,
            disk_used: r.disk,
            disk_total: r.maxdisk,
            netin: r.netin,
            netout: r.netout,
            uptime: r.uptime,
        })
    }).collect())
}

#[server]
pub async fn get_cluster_lxcs(cluster_id: String) -> Result<Vec<GuestRow>, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster client not found"))?;

    let resources = client.cluster_resources(Some("node")).await
        .map_err(|e| ServerFnError::new(format!("PVE API error: {}", e)))?;

    // Get LXCs per online node
    let mut rows = Vec::new();
    for node_r in &resources {
        if node_r.status != "online" { continue; }
        let lxcs = client.node_lxc(&node_r.node).await
            .map_err(|e| ServerFnError::new(format!("PVE LXC error: {}", e)))?;
        for l in &lxcs {
            rows.push(GuestRow {
                vmid: l.vmid,
                name: l.name.clone(),
                node: node_r.node.clone(),
                status: l.status.clone(),
                cpu_pct: l.cpu * 100.0,
                cpu_count: l.cpus as f64,
                mem_used: l.mem,
                mem_total: l.maxmem,
                disk_used: 0,
                disk_total: 0,
                netin: 0,
                netout: 0,
                uptime: l.uptime,
            });
        }
    }
    Ok(rows)
}

#[server]
pub async fn get_cluster_storage(cluster_id: String) -> Result<Vec<StorageRow>, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster client not found"))?;

    let resources = client.cluster_resources(Some("storage")).await
        .map_err(|e| ServerFnError::new(format!("PVE API error: {}", e)))?;

    Ok(resources.iter().map(|r| StorageRow {
        name: r.storage.clone().unwrap_or_else(|| r.name.clone()),
        storage_type: r.plugintype.clone().unwrap_or_default(),
        content: r.content.clone().unwrap_or_default(),
        used: r.disk,
        total: r.maxdisk,
        avail: if r.maxdisk > r.disk { r.maxdisk - r.disk } else { 0 },
        shared: r.shared == Some(1),
        active: r.status == "available",
    }).collect())
}

#[server]
pub async fn delete_cluster(cluster_id: String) -> Result<(), ServerFnError> {
    use crate::state::AppState;
    use crate::db;

    let state = expect_context::<AppState>();
    {
        let conn = state.db.lock().await;
        db::delete_cluster(&conn, &cluster_id)
            .map_err(|e| ServerFnError::new(e.to_string()))?;
    }
    state.pve_clients.write().await.remove(&cluster_id);
    Ok(())
}

#[component]
pub fn ClusterDetail() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let (confirming_delete, set_confirming_delete) = signal(false);

    let info = Resource::new(move || cluster_id(), |cid| get_cluster_info(cid));

    let on_delete = move |_| {
        let cid = cluster_id();
        leptos::task::spawn_local(async move {
            if let Ok(()) = delete_cluster(cid).await {
                #[cfg(feature = "hydrate")]
                if let Some(window) = web_sys::window() {
                    let _ = window.location().set_href("/");
                }
            }
        });
    };

    view! {
        <div>
            <Suspense fallback=|| view! { <div class="text-text-muted text-sm">"Loading cluster..."</div> }>
                {move || info.get().map(|result| match result {
                    Ok((name, api_url)) => view! {
                        <div class="flex items-center justify-between mb-4">
                            <div>
                                <h1 class="text-xl font-semibold text-text-primary">{name}</h1>
                                <p class="text-text-muted text-xs">{api_url}</p>
                            </div>
                            <div>
                                <Show
                                    when=move || confirming_delete.get()
                                    fallback=move || view! {
                                        <button
                                            on:click=move |_| set_confirming_delete.set(true)
                                            class="px-3 py-1.5 text-xs text-text-muted hover:text-accent-danger border border-border-primary rounded-md hover:border-accent-danger/50 transition-colors"
                                        >
                                            "Delete"
                                        </button>
                                    }
                                >
                                    <div class="flex items-center gap-2">
                                        <span class="text-accent-danger text-xs">"Confirm?"</span>
                                        <button
                                            on:click=on_delete
                                            class="px-3 py-1.5 text-xs bg-accent-danger text-white rounded-md"
                                        >
                                            "Yes, delete"
                                        </button>
                                        <button
                                            on:click=move |_| set_confirming_delete.set(false)
                                            class="px-3 py-1.5 text-xs text-text-muted border border-border-primary rounded-md"
                                        >
                                            "Cancel"
                                        </button>
                                    </div>
                                </Show>
                            </div>
                        </div>
                    }.into_any(),
                    Err(e) => view! {
                        <div class="text-accent-danger text-sm">{format!("Error: {}", e)}</div>
                    }.into_any(),
                })}
            </Suspense>

            <TabBar tabs=vec![
                Tab { path: format!("/clusters/{}/nodes", cluster_id()), label: "Nodes" },
                Tab { path: format!("/clusters/{}/vms", cluster_id()), label: "VMs" },
                Tab { path: format!("/clusters/{}/containers", cluster_id()), label: "Containers" },
                Tab { path: format!("/clusters/{}/storage", cluster_id()), label: "Storage" },
            ] />

            <div class="mt-4">
                <Outlet />
            </div>
        </div>
    }
}
