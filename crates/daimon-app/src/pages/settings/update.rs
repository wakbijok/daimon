use leptos::prelude::*;

#[component]
pub fn UpdateSection() -> impl IntoView {
    let current_version = env!("CARGO_PKG_VERSION");

    view! {
        <div class="bg-surface-secondary border border-border-primary rounded-lg p-4">
            <h2 class="text-sm font-semibold text-text-primary mb-3">"Update"</h2>
            <div class="flex items-center justify-between">
                <div>
                    <p class="text-sm text-text-secondary">"Current version: "<span class="text-text-primary font-mono">{current_version}</span></p>
                </div>
                <button class="px-3 py-1.5 bg-surface-tertiary border border-border-primary text-text-secondary rounded-md hover:text-text-primary text-sm transition-colors">
                    "Check for updates"
                </button>
            </div>
        </div>
    }
}
