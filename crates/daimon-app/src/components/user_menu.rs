use leptos::prelude::*;
use leptos_router::components::A;

#[component]
pub fn UserMenu(username: String, role: String) -> impl IntoView {
    let (open, set_open) = signal(false);
    let username_display = username.clone();
    let username_initial = username.chars().next().unwrap_or('U').to_uppercase().to_string();
    let dropdown_username = username.clone();
    let dropdown_role = role.clone();

    view! {
        <div class="relative">
            <button
                on:click=move |_| set_open.update(|o| *o = !*o)
                class="flex items-center gap-2 px-2 py-1.5 rounded-md hover:bg-surface-tertiary transition-colors"
            >
                <div class="w-7 h-7 rounded-full bg-accent-amber/20 text-accent-amber flex items-center justify-center text-xs font-bold">
                    {username_initial}
                </div>
                <span class="text-sm text-text-primary hidden sm:inline">{username_display}</span>
            </button>

            <Show when=move || open.get()>
                {
                    let u = dropdown_username.clone();
                    let r = dropdown_role.clone();
                    view! {
                        <div class="absolute right-0 top-full mt-1 w-48 bg-surface-secondary border border-border-primary rounded-lg shadow-lg z-50 py-1">
                            <div class="px-3 py-2 border-b border-border-primary/50">
                                <div class="text-sm font-medium text-text-primary">{u}</div>
                                <div class="text-xs text-text-muted">{r}</div>
                            </div>
                            <A
                                href="/settings"
                                attr:class="block px-3 py-2 text-sm text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition-colors"
                            >
                                "Settings"
                            </A>
                            <button
                                on:click=move |_| {
                                    // Clear cookie on client side
                                    #[cfg(feature = "hydrate")]
                                    {
                                        use wasm_bindgen::JsCast;
                                        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                                            if let Some(html_doc) = doc.dyn_ref::<web_sys::HtmlDocument>() {
                                                let _ = html_doc.set_cookie("daimon_token=; Path=/; Max-Age=0");
                                            }
                                        }
                                        if let Some(window) = web_sys::window() {
                                            let _ = window.location().set_href("/login");
                                        }
                                    }
                                }
                                class="w-full text-left px-3 py-2 text-sm text-accent-danger hover:bg-surface-tertiary transition-colors"
                            >
                                "Logout"
                            </button>
                        </div>
                    }
                }
            </Show>
        </div>
    }
}
