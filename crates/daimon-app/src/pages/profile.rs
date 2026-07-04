//! UI-1 — `/profile`: the user's OWN settings (Wak's rule: the avatar menu leads
//! here, not to system settings). Personal scope only: display identity, own
//! password change (re-authenticated), own enrolled gateway identities, theme.
//! System configuration lives in `/settings` (admin), a separate page.

use leptos::prelude::*;

use crate::components::theme::{toggle_theme, ThemeSignal};
use crate::iam::{change_my_password, my_profile};

#[component]
pub fn Profile() -> impl IntoView {
    let profile = Resource::new(|| (), |_| my_profile());

    // password change form state
    let (cur_pw, set_cur_pw) = signal(String::new());
    let (new_pw, set_new_pw) = signal(String::new());
    let (new_pw2, set_new_pw2) = signal(String::new());
    let (pw_status, set_pw_status) = signal::<Option<String>>(None);

    let change_pw = Action::new(move |(cur, new): &(String, String)| {
        let (cur, new) = (cur.clone(), new.clone());
        async move { change_my_password(cur, new).await }
    });
    Effect::new(move |_| {
        if let Some(res) = change_pw.value().get() {
            set_pw_status.set(Some(match res {
                Ok(()) => "password changed ✓".to_string(),
                Err(e) => format!("error: {e}"),
            }));
        }
    });
    let on_change_pw = move |_| {
        set_pw_status.set(None);
        if new_pw.get_untracked() != new_pw2.get_untracked() {
            set_pw_status.set(Some("new passwords do not match".into()));
            return;
        }
        change_pw.dispatch((cur_pw.get_untracked(), new_pw.get_untracked()));
        set_cur_pw.set(String::new());
        set_new_pw.set(String::new());
        set_new_pw2.set(String::new());
    };

    view! {
        <div class="space-y-6 max-w-4xl">
            <div class="flex items-baseline gap-3">
                <h1 class="text-xl font-semibold text-text-primary">"My profile"</h1>
                <span class="text-xs font-mono text-text-muted">"personal — system settings are separate"</span>
            </div>

            <Suspense fallback=|| view! { <div class="text-text-muted text-sm">"loading…"</div> }>
                {move || profile.get().map(|res| match res {
                    Ok(p) => {
                        let initial = p.username.chars().next().unwrap_or('U').to_uppercase().to_string();
                        let username = p.username.clone();
                        let roles = p.roles.clone();
                        let identities = p.identities.clone();
                        view! {
                            <div class="grid gap-4 lg:grid-cols-2">
                                // identity card
                                <div class="rounded-xl border border-border-primary bg-surface-secondary p-5 space-y-5">
                                    <div class="flex items-center gap-4">
                                        <div class="w-14 h-14 rounded-full bg-accent-amber/20 text-accent-amber flex items-center justify-center text-xl font-bold">
                                            {initial}
                                        </div>
                                        <div>
                                            <div class="text-lg font-semibold text-text-primary">{username}</div>
                                            <div class="flex gap-1.5 mt-1 flex-wrap">
                                                {roles.into_iter().map(|r| view! {
                                                    <span class="text-[10px] font-mono uppercase px-2 py-0.5 rounded-full border border-accent-amber/40 text-accent-amber">{r}</span>
                                                }).collect_view()}
                                            </div>
                                        </div>
                                    </div>

                                    <div>
                                        <h3 class="text-[11px] uppercase tracking-wider text-text-muted mb-2">"Theme"</h3>
                                        <button
                                            on:click=move |_| toggle_theme()
                                            class="px-3 py-1.5 rounded-md border border-border-primary text-sm text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition-colors"
                                        >
                                            {move || {
                                                let is_dark = use_context::<ThemeSignal>()
                                                    .map(|ThemeSignal(s)| s.get() == "dark")
                                                    .unwrap_or(true);
                                                if is_dark { "☀ Switch to light" } else { "☾ Switch to dark" }
                                            }}
                                        </button>
                                    </div>

                                    <div>
                                        <h3 class="text-[11px] uppercase tracking-wider text-text-muted mb-2">"Change password"</h3>
                                        <div class="space-y-2 max-w-xs">
                                            <input type="password" placeholder="Current password"
                                                class="w-full px-3 py-1.5 bg-surface-tertiary border border-border-primary rounded text-sm text-text-primary focus:outline-none focus:border-accent-amber"
                                                prop:value=move || cur_pw.get()
                                                on:input=move |ev| set_cur_pw.set(event_target_value(&ev)) />
                                            <input type="password" placeholder="New password (min 8 chars)"
                                                class="w-full px-3 py-1.5 bg-surface-tertiary border border-border-primary rounded text-sm text-text-primary focus:outline-none focus:border-accent-amber"
                                                prop:value=move || new_pw.get()
                                                on:input=move |ev| set_new_pw.set(event_target_value(&ev)) />
                                            <input type="password" placeholder="Repeat new password"
                                                class="w-full px-3 py-1.5 bg-surface-tertiary border border-border-primary rounded text-sm text-text-primary focus:outline-none focus:border-accent-amber"
                                                prop:value=move || new_pw2.get()
                                                on:input=move |ev| set_new_pw2.set(event_target_value(&ev)) />
                                            <div class="flex items-center gap-3">
                                                <button on:click=on_change_pw
                                                    class="px-3 py-1.5 bg-accent-amber text-surface-primary font-medium rounded text-sm">
                                                    "Update password"
                                                </button>
                                                {move || pw_status.get().map(|s| view! {
                                                    <span class="text-xs font-mono text-text-secondary">{s}</span>
                                                })}
                                            </div>
                                        </div>
                                    </div>
                                </div>

                                // identities card
                                <div class="rounded-xl border border-border-primary bg-surface-secondary p-5 space-y-3">
                                    <h3 class="text-[11px] uppercase tracking-wider text-text-muted">"My messaging identities"</h3>
                                    {if identities.is_empty() {
                                        view! {
                                            <div class="text-text-muted text-sm py-6 text-center border border-dashed border-border-primary rounded-lg">
                                                "No channel identities enrolled. An admin can bind your Telegram/Matrix handle in Settings → Channels."
                                            </div>
                                        }.into_any()
                                    } else {
                                        identities.into_iter().map(|i| {
                                            let enrolled = i.enrolled_at.chars().take(10).collect::<String>();
                                            view! {
                                                <div class="flex items-center gap-3 px-3 py-2.5 rounded-lg border border-border-primary bg-surface-tertiary/40">
                                                    <span class="text-[10px] font-mono uppercase px-2 py-0.5 rounded-full border border-blue-400/40 text-blue-300">{i.channel}</span>
                                                    <div class="flex-1 min-w-0">
                                                        <div class="text-sm text-text-primary font-mono truncate">{i.platform_handle}</div>
                                                        <div class="text-[10px] font-mono text-text-muted">{format!("enrolled {enrolled} · approve-over-chat ready")}</div>
                                                    </div>
                                                    <span class="text-[10px] font-mono text-emerald-400">"BOUND"</span>
                                                </div>
                                            }
                                        }).collect_view().into_any()
                                    }}
                                    <p class="text-[11px] text-text-muted pt-2">
                                        "These identities let you chat with daimon and approve/deny plans from your messaging apps. Binding and removal are admin actions (fail-closed)."
                                    </p>
                                </div>
                            </div>
                        }.into_any()
                    }
                    Err(e) => view! {
                        <div class="text-accent-danger text-sm">{format!("error: {e}")}</div>
                    }.into_any(),
                })}
            </Suspense>
        </div>
    }
}
