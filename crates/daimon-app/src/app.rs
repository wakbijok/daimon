use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{ParentRoute, Redirect, Route, Router, Routes},
    ParamSegment, StaticSegment,
};

use crate::components::layout::Layout;
use crate::pages::{
    admin::{AdminApprovals, AdminAudit, AdminMemory, AdminObserver, AdminPlans},
    class_dashboard::{Infrastructure, KubernetesDash, NetworkDash},
    dashboard::Dashboard,
    incident_detail::IncidentDetail,
    incidents::Incidents,
    login::Login,
    profile::Profile,
    settings::Settings,
    topology::Topology,
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
                    // ---- Operate: the feature dashboards (UI-1, console v2) ----
                    <Route path=StaticSegment("") view=Dashboard />
                    <Route path=StaticSegment("incidents") view=Incidents />
                    <Route path=(StaticSegment("incidents"), ParamSegment("id")) view=IncidentDetail />
                    <Route path=StaticSegment("infrastructure") view=Infrastructure />
                    <Route path=StaticSegment("network") view=NetworkDash />
                    <Route path=StaticSegment("kubernetes") view=KubernetesDash />
                    // NOTE: NOT "/metrics" — that path is the unauthenticated
                    // Prometheus scrape endpoint registered in main.rs (self-
                    // observability). A Leptos route on it panics the router at
                    // boot ("Overlapping method route"). Operate surface = /monitoring.
                    <Route path=StaticSegment("monitoring") view=AdminObserver />
                    <Route path=StaticSegment("topology") view=Topology />
                    <Route path=StaticSegment("plans") view=AdminPlans />
                    <Route path=StaticSegment("approvals") view=AdminApprovals />
                    <Route path=StaticSegment("audit") view=AdminAudit />
                    <Route path=StaticSegment("memory") view=AdminMemory />

                    // ---- Personal ----
                    <Route path=StaticSegment("profile") view=Profile />

                    // ---- System configuration (one home) ----
                    <Route path=StaticSegment("settings") view=Settings />

                    // ---- Legacy /admin/* redirects (UI-1) ------------------
                    // Operational pages became top-level dashboards; config
                    // pages live in Settings. Bookmarks keep working.
                    <Route path=(StaticSegment("admin"), StaticSegment("plans"))
                        view=|| view! { <Redirect path="/plans"/> } />
                    <Route path=(StaticSegment("admin"), StaticSegment("approvals"))
                        view=|| view! { <Redirect path="/approvals"/> } />
                    <Route path=(StaticSegment("admin"), StaticSegment("audit"))
                        view=|| view! { <Redirect path="/audit"/> } />
                    <Route path=(StaticSegment("admin"), StaticSegment("memory"))
                        view=|| view! { <Redirect path="/memory"/> } />
                    <Route path=(StaticSegment("admin"), StaticSegment("observer"))
                        view=|| view! { <Redirect path="/monitoring"/> } />
                    <Route path=(StaticSegment("admin"), StaticSegment("credentials"))
                        view=|| view! { <Redirect path="/settings"/> } />
                    <Route path=(StaticSegment("admin"), StaticSegment("targets"))
                        view=|| view! { <Redirect path="/settings"/> } />
                    <Route path=(StaticSegment("admin"), StaticSegment("iam"))
                        view=|| view! { <Redirect path="/settings"/> } />
                </ParentRoute>
            </Routes>
        </Router>
    }
}
