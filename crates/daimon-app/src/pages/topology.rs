//! UI-5 — `/topology`: the fleet topology view. This UI-1 stub reserves the
//! route; UI-5 (same arc) replaces it with the layered SVG render fed by
//! `topology_snapshot`.

use leptos::prelude::*;

#[component]
pub fn Topology() -> impl IntoView {
    view! {
        <div class="space-y-5">
            <h1 class="text-xl font-semibold text-text-primary">"Topology"</h1>
            <div class="text-text-muted text-sm py-10 text-center border border-dashed border-border-primary rounded-xl">
                "Topology view lands in UI-5 of this arc."
            </div>
        </div>
    }
}
