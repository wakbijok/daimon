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
    settings::Settings,
    admin::{
        AdminApprovals, AdminAudit, AdminCredentials, AdminMemory, AdminObserver, AdminPlans,
        AdminTargets,
    },
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

                    // Phase 8 — operator approval inbox + blast-radius
                    <Route
                        path=(StaticSegment("admin"), StaticSegment("approvals"))
                        view=AdminApprovals
                    />
                </ParentRoute>
            </Routes>
        </Router>
    }
}
