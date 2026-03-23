use leptos::prelude::*;
use leptos_router::components::Outlet;
use leptos_router::hooks::use_params_map;
use crate::components::tabs::{Tab, TabBar};
use crate::components::table::MetricRow;

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
pub async fn get_cluster_nodes(cluster_id: String) -> Result<Vec<MetricRow>, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster client not found"))?;

    let nodes = client.nodes().await.map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(nodes.iter().map(|n| {
        let ram_pct = if n.maxmem > 0 { (n.mem as f64 / n.maxmem as f64) * 100.0 } else { 0.0 };
        MetricRow {
            name: n.node.clone(),
            sub: format!("{} CPU(s)", n.maxcpu),
            cpu_pct: n.cpu * 100.0,
            ram_pct,
            status: n.status.clone(),
        }
    }).collect())
}

#[server]
pub async fn get_cluster_vms(cluster_id: String) -> Result<Vec<MetricRow>, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster client not found"))?;

    let nodes = client.nodes().await.map_err(|e| ServerFnError::new(e.to_string()))?;
    let mut rows = Vec::new();
    for node in &nodes {
        if node.status != "online" { continue; }
        let vms = client.node_qemu(&node.node).await.map_err(|e| ServerFnError::new(e.to_string()))?;
        for vm in &vms {
            let ram_pct = if vm.maxmem > 0 { (vm.mem as f64 / vm.maxmem as f64) * 100.0 } else { 0.0 };
            rows.push(MetricRow {
                name: vm.name.clone(),
                sub: format!("VMID {} on {}", vm.vmid, node.node),
                cpu_pct: vm.cpu * 100.0,
                ram_pct,
                status: vm.status.clone(),
            });
        }
    }
    Ok(rows)
}

#[server]
pub async fn get_cluster_lxcs(cluster_id: String) -> Result<Vec<MetricRow>, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster client not found"))?;

    let nodes = client.nodes().await.map_err(|e| ServerFnError::new(e.to_string()))?;
    let mut rows = Vec::new();
    for node in &nodes {
        if node.status != "online" { continue; }
        let lxcs = client.node_lxc(&node.node).await.map_err(|e| ServerFnError::new(e.to_string()))?;
        for ct in &lxcs {
            let ram_pct = if ct.maxmem > 0 { (ct.mem as f64 / ct.maxmem as f64) * 100.0 } else { 0.0 };
            rows.push(MetricRow {
                name: ct.name.clone(),
                sub: format!("CTID {} on {}", ct.vmid, node.node),
                cpu_pct: ct.cpu * 100.0,
                ram_pct,
                status: ct.status.clone(),
            });
        }
    }
    Ok(rows)
}

#[server]
pub async fn get_cluster_storage(cluster_id: String) -> Result<Vec<MetricRow>, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    let clients = state.pve_clients.read().await;
    let client = clients.get(&cluster_id)
        .ok_or_else(|| ServerFnError::new("Cluster client not found"))?;

    let stores = client.storage().await.map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(stores.iter().map(|s| {
        let used_pct = if s.total > 0 { (s.used as f64 / s.total as f64) * 100.0 } else { 0.0 };
        MetricRow {
            name: s.storage.clone(),
            sub: s.storage_type.clone(),
            cpu_pct: used_pct,
            ram_pct: 0.0,
            status: if s.active == Some(1) { "online".to_string() } else { "inactive".to_string() },
        }
    }).collect())
}

#[component]
pub fn ClusterDetail() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();

    let info = Resource::new(
        move || cluster_id(),
        |cid| get_cluster_info(cid),
    );

    view! {
        <div>
            <Suspense fallback=|| view! { <div class="text-text-muted text-sm">"Loading cluster..."</div> }>
                {move || info.get().map(|result| match result {
                    Ok((name, api_url)) => view! {
                        <div class="mb-4">
                            <h1 class="text-xl font-semibold text-text-primary">{name}</h1>
                            <p class="text-text-muted text-xs">{api_url}</p>
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

            <Outlet />
        </div>
    }
}
