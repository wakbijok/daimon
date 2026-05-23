use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{ParentRoute, Route, Router, Routes},
    ParamSegment, StaticSegment,
};

use crate::components::layout::Layout;
use crate::pages::{
    login::Login,
    dashboard::Dashboard,
    incidents::Incidents,
    incident_detail::IncidentDetail,
    cluster::{
        detail::ClusterDetail,
        add::AddCluster,
        nodes::Nodes,
        vms::Vms,
        containers::Containers,
        storage::Storage,
        node_detail::{NodeDetail, NodeOverview},
        node_charts::NodeCharts,
        vm_detail::{VmDetail, VmOverview},
        vm_charts::VmCharts,
        lxc_detail::{LxcDetail, LxcOverview},
        lxc_charts::LxcCharts,
        storage_detail::{StorageDetail, StorageOverview},
        storage_usage::StorageUsage,
        storage_charts::StorageCharts,
        agent_placeholder::AgentPlaceholder,
    },
    settings::Settings,
    admin::{AdminAudit, AdminCredentials, AdminMemory, AdminObserver, AdminPlans, AdminTargets},
};

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone() />
                <HydrationScripts options/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/daimon.css"/>
        <Title text="daimon"/>
        <Router>
            <Routes fallback=|| "Page not found.".into_view()>
                // Login (no layout wrapper)
                <Route path=StaticSegment("login") view=Login />

                // All other routes wrapped in Layout (sidebar + top bar + main)
                <ParentRoute path=StaticSegment("") view=Layout>
                    <Route path=StaticSegment("") view=Dashboard />
                    <Route path=StaticSegment("incidents") view=Incidents />
                    <Route path=(StaticSegment("incidents"), ParamSegment("id")) view=IncidentDetail />
                    <Route path=(StaticSegment("clusters"), StaticSegment("add")) view=AddCluster />
                    <ParentRoute path=(StaticSegment("clusters"), ParamSegment("cluster_id")) view=ClusterDetail>
                        <Route path=StaticSegment("nodes") view=Nodes />
                        <Route path=StaticSegment("vms") view=Vms />
                        <Route path=StaticSegment("containers") view=Containers />
                        <Route path=StaticSegment("storage") view=Storage />
                        <Route path=StaticSegment("") view=Nodes />
                    </ParentRoute>
                    // Storage detail (more specific path — must be before node detail)
                    <ParentRoute path=(StaticSegment("clusters"), ParamSegment("cluster_id"), StaticSegment("nodes"), ParamSegment("node_name"), StaticSegment("storage"), ParamSegment("storage_name")) view=StorageDetail>
                        <Route path=StaticSegment("") view=StorageOverview />
                        <Route path=StaticSegment("devices") view=|| view! { <AgentPlaceholder tab_name="Devices" description="Per-disk status, temperature, and SMART data." /> } />
                        <Route path=StaticSegment("usage") view=StorageUsage />
                        <Route path=StaticSegment("charts") view=StorageCharts />
                    </ParentRoute>

                    // Node detail
                    <ParentRoute path=(StaticSegment("clusters"), ParamSegment("cluster_id"), StaticSegment("nodes"), ParamSegment("node_name")) view=NodeDetail>
                        <Route path=StaticSegment("") view=NodeOverview />
                        <Route path=StaticSegment("hardware") view=|| view! { <AgentPlaceholder tab_name="Hardware" description="IPMI sensors, motherboard info, and BIOS details." /> } />
                        <Route path=StaticSegment("raid") view=|| view! { <AgentPlaceholder tab_name="RAID" description="RAID controller, virtual drives, physical drives, cache policy, and BBU health." /> } />
                        <Route path=StaticSegment("disks") view=|| view! { <AgentPlaceholder tab_name="Disks" description="Per-disk SMART data, temperature, and utilization." /> } />
                        <Route path=StaticSegment("storage") view=|| view! { <AgentPlaceholder tab_name="Storage" description="Local mounts, NFS/CIFS/iSCSI/FC, and multipath status." /> } />
                        <Route path=StaticSegment("network") view=|| view! { <AgentPlaceholder tab_name="Network" description="Per-NIC traffic, errors, drops, bond/bridge status." /> } />
                        <Route path=StaticSegment("services") view=|| view! { <AgentPlaceholder tab_name="Services" description="PVE daemon states: pvedaemon, pveproxy, corosync, ceph." /> } />
                        <Route path=StaticSegment("charts") view=NodeCharts />
                    </ParentRoute>

                    // VM detail
                    <ParentRoute path=(StaticSegment("clusters"), ParamSegment("cluster_id"), StaticSegment("vms"), ParamSegment("vmid")) view=VmDetail>
                        <Route path=StaticSegment("") view=VmOverview />
                        <Route path=StaticSegment("processes") view=|| view! { <AgentPlaceholder tab_name="Processes" description="Top processes by CPU/RAM, zombie count." /> } />
                        <Route path=StaticSegment("services") view=|| view! { <AgentPlaceholder tab_name="Services" description="systemd units and Docker containers." /> } />
                        <Route path=StaticSegment("network") view=|| view! { <AgentPlaceholder tab_name="Network" description="Listening ports, connections, per-interface traffic." /> } />
                        <Route path=StaticSegment("logs") view=|| view! { <AgentPlaceholder tab_name="Logs" description="Recent journal entries, filterable by unit/priority." /> } />
                        <Route path=StaticSegment("charts") view=VmCharts />
                    </ParentRoute>

                    // LXC detail
                    <ParentRoute path=(StaticSegment("clusters"), ParamSegment("cluster_id"), StaticSegment("containers"), ParamSegment("vmid")) view=LxcDetail>
                        <Route path=StaticSegment("") view=LxcOverview />
                        <Route path=StaticSegment("processes") view=|| view! { <AgentPlaceholder tab_name="Processes" description="Top processes by CPU/RAM, zombie count." /> } />
                        <Route path=StaticSegment("services") view=|| view! { <AgentPlaceholder tab_name="Services" description="systemd units and Docker containers." /> } />
                        <Route path=StaticSegment("network") view=|| view! { <AgentPlaceholder tab_name="Network" description="Listening ports, connections, per-interface traffic." /> } />
                        <Route path=StaticSegment("logs") view=|| view! { <AgentPlaceholder tab_name="Logs" description="Recent journal entries, filterable by unit/priority." /> } />
                        <Route path=StaticSegment("charts") view=LxcCharts />
                    </ParentRoute>

                    <Route path=StaticSegment("settings") view=Settings />

                    // Admin routes (Phase 2b #12/#13/#14)
                    <Route
                        path=(StaticSegment("admin"), StaticSegment("credentials"))
                        view=AdminCredentials
                    />
                    <Route
                        path=(StaticSegment("admin"), StaticSegment("targets"))
                        view=AdminTargets
                    />
                    <Route
                        path=(StaticSegment("admin"), StaticSegment("audit"))
                        view=AdminAudit
                    />
                    <Route
                        path=(StaticSegment("admin"), StaticSegment("memory"))
                        view=AdminMemory
                    />

                    // Phase 6 D1 — plans + DAG inspector
                    <Route
                        path=(StaticSegment("admin"), StaticSegment("plans"))
                        view=AdminPlans
                    />

                    // Phase 7 — observer (anomalies + metric streams)
                    <Route
                        path=(StaticSegment("admin"), StaticSegment("observer"))
                        view=AdminObserver
                    />
                </ParentRoute>
            </Routes>
        </Router>
    }
}
