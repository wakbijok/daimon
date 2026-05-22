use leptos::prelude::*;

/// Stub page for `/admin/targets` (#13) and `/admin/audit` (#14) until those
/// land. Renders a single line so the route + nav entry don't 404.
#[component]
pub fn AdminPlaceholder(
    #[prop(into)] title: String,
    #[prop(into)] note: String,
) -> impl IntoView {
    let title_for_view = title.clone();
    let note_for_view = note.clone();
    view! {
        <div>
            <h1 class="text-xl font-semibold text-text-primary mb-2">{title_for_view}</h1>
            <p class="text-text-muted text-sm">{note_for_view}</p>
        </div>
    }
}
