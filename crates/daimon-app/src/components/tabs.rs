use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_location;

pub struct Tab {
    pub path: String,
    pub label: &'static str,
}

#[component]
pub fn TabBar(tabs: Vec<Tab>) -> impl IntoView {
    let location = use_location();
    let pathname = move || location.pathname.get();

    view! {
        <div class="flex gap-1 border-b border-border-primary mb-4">
            {tabs.into_iter().map(|tab| {
                let path = tab.path.clone();
                let path2 = tab.path.clone();
                let label = tab.label;
                view! {
                    <A
                        href=path
                        attr:class=move || format!(
                            "px-3 py-2 text-sm font-medium transition-colors -mb-px {}",
                            if pathname().ends_with(&path2) || pathname().ends_with(&format!("{}/", path2)) {
                                "text-accent-amber border-b-2 border-accent-amber"
                            } else {
                                "text-text-muted hover:text-text-secondary"
                            }
                        )
                    >
                        {label}
                    </A>
                }
            }).collect_view()}
        </div>
    }
}
