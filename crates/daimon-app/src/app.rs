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
    cluster::{nodes::Nodes, vms::Vms, containers::Containers, storage::Storage},
    settings::Settings,
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

                // All other routes wrapped in Layout (sidebar + main)
                <ParentRoute path=StaticSegment("") view=Layout>
                    <Route path=StaticSegment("") view=Dashboard />
                    <Route path=StaticSegment("incidents") view=Incidents />
                    <Route path=(StaticSegment("incidents"), ParamSegment("id")) view=IncidentDetail />
                    <Route path=(StaticSegment("cluster"), StaticSegment("nodes")) view=Nodes />
                    <Route path=(StaticSegment("cluster"), StaticSegment("vms")) view=Vms />
                    <Route path=(StaticSegment("cluster"), StaticSegment("containers")) view=Containers />
                    <Route path=(StaticSegment("cluster"), StaticSegment("storage")) view=Storage />
                    <Route path=StaticSegment("settings") view=Settings />
                </ParentRoute>
            </Routes>
        </Router>
    }
}
