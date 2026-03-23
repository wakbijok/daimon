pub mod update;

use leptos::prelude::*;
use update::UpdateSection;

#[component]
pub fn Settings() -> impl IntoView {
    view! {
        <div>
            <h1 class="text-xl font-semibold text-text-primary mb-6">"Settings"</h1>
            <div class="space-y-6">
                <UpdateSection />
            </div>
        </div>
    }
}
