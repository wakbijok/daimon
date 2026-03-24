use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_location;

pub struct DetailTab {
    pub label: &'static str,
    pub path: String,
    pub requires_agent: bool,
}

#[component]
pub fn DetailLayout(
    title: String,
    #[prop(optional)]
    subtitle: Option<String>,
    tabs: Vec<DetailTab>,
    children: Children,
) -> impl IntoView {
    let location = use_location();
    let pathname = move || location.pathname.get();

    view! {
        <div>
            <div class="mb-4">
                <h1 class="text-xl font-semibold text-text-primary">{title}</h1>
                {subtitle.map(|s| view! {
                    <p class="text-xs text-text-muted">{s}</p>
                })}
            </div>

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
                                if pathname().ends_with(&path2)
                                    || pathname().ends_with(&format!("{}/", path2))
                                {
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

            <div>
                {children()}
            </div>
        </div>
    }
}
