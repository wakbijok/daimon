use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

#[component]
pub fn IncidentDetail() -> impl IntoView {
    let params = use_params_map();
    let id = move || params.get().get("id").unwrap_or_default();

    view! {
        <div>
            <h1 class="text-xl font-semibold text-text-primary">"Incident " {id}</h1>
            <p class="text-text-muted mt-2 text-sm">"Incident detail view — coming soon"</p>
        </div>
    }
}
