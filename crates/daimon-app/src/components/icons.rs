use leptos::prelude::*;

#[component]
pub fn Icon(
    #[prop(into)] d: String,
    #[prop(default = "w-4 h-4".to_string(), into)] class: String,
) -> impl IntoView {
    view! {
        <svg class=class fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d=d />
        </svg>
    }
}
