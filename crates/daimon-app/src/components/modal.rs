use leptos::prelude::*;

/// Reusable modal wrapper. Backdrop click closes; Esc closes via on:keydown
/// on the focusable outer div (no window-level listener — Leptos cleans up
/// when `open` flips false).
#[component]
pub fn Modal(
    #[prop(into)] title: String,
    open: RwSignal<bool>,
    #[prop(optional, default = "max-w-md")] max_width: &'static str,
    children: ChildrenFn,
) -> impl IntoView {
    let title_for_view = title.clone();
    let close_on_backdrop = move |_| open.set(false);
    let close_on_esc = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Escape" {
            open.set(false);
        }
    };
    let stop = move |ev: leptos::ev::MouseEvent| ev.stop_propagation();

    view! {
        <Show when=move || open.get()>
            <div
                class="fixed inset-0 bg-black/60 flex items-center justify-center z-50"
                on:click=close_on_backdrop
                on:keydown=close_on_esc
                tabindex="-1"
                autofocus
            >
                <div
                    class=format!("bg-surface-secondary border border-border-primary rounded-lg shadow-xl w-full {} mx-4 max-h-[90vh] overflow-y-auto", max_width)
                    on:click=stop
                >
                    <div class="flex items-center justify-between px-5 py-3 border-b border-border-primary">
                        <h2 class="text-base font-semibold text-text-primary">{title_for_view.clone()}</h2>
                        <button
                            type="button"
                            on:click=move |_| open.set(false)
                            class="w-7 h-7 flex items-center justify-center rounded-md text-text-muted hover:text-text-primary hover:bg-surface-tertiary transition-colors"
                            aria-label="Close"
                        >
                            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
                            </svg>
                        </button>
                    </div>
                    <div class="px-5 py-4">
                        {children()}
                    </div>
                </div>
            </div>
        </Show>
    }
}
