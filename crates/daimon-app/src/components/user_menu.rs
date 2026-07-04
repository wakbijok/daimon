//! UI-1 — the avatar menu (console v2, Wak's rule): "My profile" is the user's
//! OWN settings page; "System settings" is a separate ADMIN entry — no more
//! redirecting the profile into system configuration.

use crate::components::theme::{toggle_theme, ThemeSignal};
use leptos::prelude::*;
use leptos_router::components::A;

#[component]
pub fn UserMenu(username: String, role: String) -> impl IntoView {
    let (open, set_open) = signal(false);
    let username_display = username.clone();
    let username_initial = username.chars().next().unwrap_or('U').to_uppercase().to_string();
    let dropdown_username = username.clone();
    let dropdown_role = role.clone();
    // Menu-level gate only — /settings server fns stay require_admin regardless.
    let is_admin = role == "admin";

    // FR-IAM-19 / NFR-SEC-03: logout deletes the server session (not just the
    // cookie). Redirect to /login only after the server-fn resolves.
    let logout = ServerAction::<crate::iam::Logout>::new();
    Effect::new(move || {
        if logout.value().get().is_some() {
            #[cfg(feature = "hydrate")]
            if let Some(window) = web_sys::window() {
                let _ = window.location().set_href("/login");
            }
        }
    });

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
                        <div class="absolute right-0 top-full mt-1 w-52 bg-surface-secondary border border-border-primary rounded-lg shadow-lg z-50 py-1">
                            <div class="px-3 py-2 border-b border-border-primary/50">
                                <div class="text-sm font-medium text-text-primary">{u}</div>
                                <div class="text-[10px] font-mono uppercase tracking-wider text-accent-amber">{r}</div>
                            </div>

                            // Personal
                            <A
                                href="/profile"
                                attr:class="block px-3 py-2 text-sm text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition-colors"
                            >
                                "My profile"
                            </A>
                            <button
                                on:click=move |_| toggle_theme()
                                class="w-full text-left px-3 py-2 text-sm text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition-colors"
                            >
                                {move || {
                                    let is_dark = use_context::<ThemeSignal>()
                                        .map(|ThemeSignal(s)| s.get() == "dark")
                                        .unwrap_or(true);
                                    if is_dark { "☀ Light mode" } else { "☾ Dark mode" }
                                }}
                            </button>

                            // Admin-only: SYSTEM settings — a separate page, not the profile.
                            <Show when=move || is_admin>
                                <div class="border-t border-border-primary/50 mt-1 pt-1">
                                    <A
                                        href="/settings"
                                        attr:class="flex items-center px-3 py-2 text-sm text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition-colors"
                                    >
                                        <span class="flex-1">"System settings"</span>
                                        <span class="text-[9px] font-mono uppercase px-1.5 py-0.5 rounded-full border border-accent-amber/40 text-accent-amber">"admin"</span>
                                    </A>
                                </div>
                            </Show>

                            <div class="border-t border-border-primary/50 mt-1 pt-1">
                                <button
                                    on:click=move |_| { logout.dispatch(crate::iam::Logout {}); }
                                    class="w-full text-left px-3 py-2 text-sm text-accent-danger hover:bg-surface-tertiary transition-colors"
                                >
                                    "Sign out"
                                </button>
                            </div>
                        </div>
                    }
                }
            </Show>
        </div>
    }
}
