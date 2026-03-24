use leptos::prelude::*;
use leptos_router::components::Outlet;
use leptos_router::hooks::use_params_map;
use serde::{Deserialize, Serialize};
use crate::components::tabs::{Tab, TabBar};
use crate::components::table::{NodeRow, GuestRow, StorageRow};

/// Guest entry with resource type info, for node overview guest list
#[derive(Clone, Serialize, Deserialize)]
pub struct NodeGuest {
    pub vmid: u32,
    pub name: String,
    pub guest_type: String, // "qemu" or "lxc"
    pub status: String,
    pub cpu_pct: f64,
    pub mem_used: u64,
    pub mem_total: u64,
    pub uptime: u64,
}

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
        node: r.node.clone(),
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

// ---------------------------------------------------------------------------
// Detail-level server functions (node/guest/storage status, RRD, config)
// ---------------------------------------------------------------------------

#[server]
pub async fn get_node_status(cluster_id: String, node_name: String) -> Result<daimon_pve::PveNodeStatus, ServerFnError> {
    use crate::state::AppState;
    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster not found"))?;

    client.node_status(&node_name).await
        .map_err(|e| ServerFnError::new(format!("Status error: {}", e)))
}

#[server]
pub async fn get_node_rrd(cluster_id: String, node_name: String, timeframe: String) -> Result<Vec<daimon_pve::RrdDataPoint>, ServerFnError> {
    use crate::state::AppState;
    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster not found"))?;

    let tf = match timeframe.as_str() {
        "hour" => daimon_pve::RrdTimeframe::Hour,
        "day" => daimon_pve::RrdTimeframe::Day,
        "week" => daimon_pve::RrdTimeframe::Week,
        "month" => daimon_pve::RrdTimeframe::Month,
        "year" => daimon_pve::RrdTimeframe::Year,
        _ => daimon_pve::RrdTimeframe::Hour,
    };

    client.node_rrddata(&node_name, tf).await
        .map_err(|e| ServerFnError::new(format!("RRD error: {}", e)))
}

/// Get list of guests running on a specific node (with type info for routing)
#[server]
pub async fn get_node_guests(cluster_id: String, node_name: String) -> Result<Vec<NodeGuest>, ServerFnError> {
    use crate::state::AppState;
    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster not found"))?;

    let resources = client.cluster_resources(None).await
        .map_err(|e| ServerFnError::new(format!("PVE error: {}", e)))?;

    Ok(resources.iter().filter_map(|r| {
        if r.node != node_name { return None; }
        if r.resource_type != "qemu" && r.resource_type != "lxc" { return None; }
        r.vmid.map(|vmid| NodeGuest {
            vmid,
            name: r.name.clone(),
            guest_type: r.resource_type.clone(),
            status: r.status.clone(),
            cpu_pct: r.cpu * 100.0,
            mem_used: r.mem,
            mem_total: r.maxmem,
            uptime: r.uptime,
        })
    }).collect())
}

/// Find which node a guest (VM/LXC) is running on. Returns (node_name, resource_type).
#[server]
pub async fn find_guest_node(cluster_id: String, vmid: u32) -> Result<(String, String), ServerFnError> {
    use crate::state::AppState;
    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster not found"))?;

    let resources = client.cluster_resources(None).await
        .map_err(|e| ServerFnError::new(format!("PVE error: {}", e)))?;

    resources.iter()
        .find(|r| r.vmid == Some(vmid))
        .map(|r| (r.node.clone(), r.resource_type.clone()))
        .ok_or_else(|| ServerFnError::new(format!("Guest {} not found", vmid)))
}

#[server]
pub async fn get_guest_rrd(cluster_id: String, node: String, vmid: u32, guest_type: String, timeframe: String) -> Result<Vec<daimon_pve::RrdDataPoint>, ServerFnError> {
    use crate::state::AppState;
    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster not found"))?;

    let tf = match timeframe.as_str() {
        "hour" => daimon_pve::RrdTimeframe::Hour,
        "day" => daimon_pve::RrdTimeframe::Day,
        "week" => daimon_pve::RrdTimeframe::Week,
        "month" => daimon_pve::RrdTimeframe::Month,
        "year" => daimon_pve::RrdTimeframe::Year,
        _ => daimon_pve::RrdTimeframe::Hour,
    };

    let result = if guest_type == "qemu" {
        client.qemu_rrddata(&node, vmid, tf).await
    } else {
        client.lxc_rrddata(&node, vmid, tf).await
    };
    result.map_err(|e| ServerFnError::new(format!("RRD error: {}", e)))
}

#[server]
pub async fn get_guest_status(cluster_id: String, node: String, vmid: u32, guest_type: String) -> Result<serde_json::Value, ServerFnError> {
    use crate::state::AppState;
    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster not found"))?;

    let result = if guest_type == "qemu" {
        let s = client.qemu_status(&node, vmid).await
            .map_err(|e| ServerFnError::new(format!("Status error: {}", e)))?;
        serde_json::to_value(s).map_err(|e| ServerFnError::new(e.to_string()))?
    } else {
        let s = client.lxc_status(&node, vmid).await
            .map_err(|e| ServerFnError::new(format!("Status error: {}", e)))?;
        serde_json::to_value(s).map_err(|e| ServerFnError::new(e.to_string()))?
    };
    Ok(result)
}

#[server]
pub async fn get_guest_config(cluster_id: String, node: String, vmid: u32, guest_type: String) -> Result<daimon_pve::GuestConfig, ServerFnError> {
    use crate::state::AppState;
    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster not found"))?;

    let result = if guest_type == "qemu" {
        client.qemu_config(&node, vmid).await
    } else {
        client.lxc_config(&node, vmid).await
    };
    result.map_err(|e| ServerFnError::new(format!("Config error: {}", e)))
}

#[server]
pub async fn get_storage_rrd(cluster_id: String, node_name: String, storage_name: String, timeframe: String) -> Result<Vec<daimon_pve::RrdDataPoint>, ServerFnError> {
    use crate::state::AppState;
    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster not found"))?;

    let tf = match timeframe.as_str() {
        "hour" => daimon_pve::RrdTimeframe::Hour,
        "day" => daimon_pve::RrdTimeframe::Day,
        "week" => daimon_pve::RrdTimeframe::Week,
        "month" => daimon_pve::RrdTimeframe::Month,
        "year" => daimon_pve::RrdTimeframe::Year,
        _ => daimon_pve::RrdTimeframe::Hour,
    };

    client.storage_rrddata(&node_name, &storage_name, tf).await
        .map_err(|e| ServerFnError::new(format!("RRD error: {}", e)))
}

/// Get storage info from cluster resources for a specific storage on a node
#[server]
pub async fn get_storage_status(cluster_id: String, node_name: String, storage_name: String) -> Result<StorageRow, ServerFnError> {
    use crate::state::AppState;
    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster not found"))?;

    let resources = client.cluster_resources(Some("storage")).await
        .map_err(|e| ServerFnError::new(format!("PVE error: {}", e)))?;

    resources.iter()
        .find(|r| r.node == node_name && r.storage.as_deref() == Some(&storage_name))
        .map(|r| StorageRow {
            name: r.storage.clone().unwrap_or_else(|| r.name.clone()),
            node: r.node.clone(),
            storage_type: r.plugintype.clone().unwrap_or_default(),
            content: r.content.clone().unwrap_or_default(),
            used: r.disk,
            total: r.maxdisk,
            avail: if r.maxdisk > r.disk { r.maxdisk - r.disk } else { 0 },
            shared: r.shared == Some(1),
            active: r.status == "available",
        })
        .ok_or_else(|| ServerFnError::new(format!("Storage {} not found on {}", storage_name, node_name)))
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
