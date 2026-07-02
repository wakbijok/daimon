//! Phase 8 — `/settings` 9-tab operator surface.
//! Reachable via the sidebar bottom-left button and the user-menu
//! dropdown. The admin sidebar deliberately does NOT list a separate
//! Settings entry — one entry point only.
//!
//! Tabs:
//! 1. Identity      — tenant + user + JWT
//! 2. Connections   — backend URLs (read-only display today)
//! 3. LLM Providers — Anthropic / OpenAI / Ollama + per-role defaults
//! 4. Guard         — approval timeout, KILL path, blast-radius depth
//! 5. Observer      — poll intervals, named-query library (placeholder)
//! 6. RAG & Memory  — embedding model, chunk size, reranker, budget
//! 7. Vault & KMS   — KMS backend, envelope path, DEK rotation
//! 8. System        — read-only dashboard
//! 9. Update        — channel, check, apply

use leptos::prelude::*;

use crate::admin_settings::{
    apply_update, cancel_update, check_for_update, get_system_info, get_update_state,
    list_settings, set_setting, SettingRow, SystemInfo, UpdateState,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Identity,
    Connections,
    Llm,
    Guard,
    Observer,
    Rag,
    Vault,
    System,
    Update,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Identity => "Identity",
            Tab::Connections => "Connections",
            Tab::Llm => "LLM Providers",
            Tab::Guard => "Guard",
            Tab::Observer => "Observer",
            Tab::Rag => "RAG & Memory",
            Tab::Vault => "Vault & KMS",
            Tab::System => "System",
            Tab::Update => "Update",
        }
    }
    fn prefix(self) -> &'static str {
        match self {
            Tab::Identity => "identity.",
            Tab::Connections => "connections.",
            Tab::Llm => "llm.",
            Tab::Guard => "guard.",
            Tab::Observer => "observer.",
            Tab::Rag => "rag.",
            Tab::Vault => "vault.",
            Tab::System => "",
            Tab::Update => "update.",
        }
    }
}

const TAB_ORDER: [Tab; 9] = [
    Tab::Identity,
    Tab::Connections,
    Tab::Llm,
    Tab::Guard,
    Tab::Observer,
    Tab::Rag,
    Tab::Vault,
    Tab::System,
    Tab::Update,
];

#[component]
pub fn Settings() -> impl IntoView {
    let (tab, set_tab) = signal(Tab::Identity);

    view! {
        <div class="flex h-full">
            <nav class="w-56 border-r border-border-primary p-2 space-y-1 shrink-0">
                <h1 class="text-sm uppercase tracking-wider text-text-secondary px-2 py-2">
                    "Settings"
                </h1>
                {TAB_ORDER.iter().map(|t| {
                    let this = *t;
                    let label = this.label();
                    let active = Memo::new(move |_| tab.get() == this);
                    view! {
                        <button
                            on:click=move |_| set_tab.set(this)
                            class=move || format!(
                                "w-full text-left px-3 py-1.5 rounded text-sm {}",
                                if active.get() {
                                    "bg-accent-amber/20 text-accent-amber"
                                } else {
                                    "text-text-primary hover:bg-surface-secondary"
                                }
                            )
                        >
                            {label}
                        </button>
                    }
                }).collect_view()}
            </nav>
            <div class="flex-1 overflow-y-auto p-6">
                {move || match tab.get() {
                    Tab::System => view! { <SystemTab /> }.into_any(),
                    Tab::Update => view! { <UpdateTab /> }.into_any(),
                    other => view! { <KvTab tab=other /> }.into_any(),
                }}
            </div>
        </div>
    }
}

// ---- Generic key/value tab (Identity, Connections, LLM, Guard, Observer,
//      RAG, Vault) -----------------------------------------------------------

#[component]
fn KvTab(tab: Tab) -> impl IntoView {
    let label = tab.label();
    let prefix = tab.prefix();
    let settings = Resource::new(move || prefix, |p| list_settings(p.to_string()));
    let (status, set_status) = signal::<Option<String>>(None);
    let (new_key, set_new_key) = signal(String::new());
    let (new_val, set_new_val) = signal(String::new());
    let (new_secret, set_new_secret) = signal(false);

    let save_action = Action::new(move |args: &(String, serde_json::Value, bool)| {
        let (k, v, s) = args.clone();
        async move {
            match set_setting(k, v, s).await {
                Ok(_) => Some("saved".to_string()),
                Err(e) => Some(format!("error: {e}")),
            }
        }
    });

    Effect::new(move |_| {
        if let Some(m) = save_action.value().get().flatten() {
            set_status.set(Some(m));
            settings.refetch();
        }
    });

    let prefix_for_save = prefix;
    let on_save_new = move |_| {
        let k = new_key.get().trim().to_string();
        if k.is_empty() {
            set_status.set(Some("key is required".into()));
            return;
        }
        let full_key = if k.starts_with(prefix_for_save) {
            k
        } else {
            format!("{prefix_for_save}{k}")
        };
        let raw = new_val.get();
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => serde_json::Value::String(raw),
        };
        save_action.dispatch((full_key, value, new_secret.get()));
        set_new_key.set(String::new());
        set_new_val.set(String::new());
        set_new_secret.set(false);
    };

    view! {
        <div class="space-y-4 max-w-3xl">
            <div class="flex items-center justify-between">
                <h2 class="text-lg font-semibold text-text-primary">{label}</h2>
                {move || status.get().map(|s| view! {
                    <span class="text-xs font-mono text-text-secondary">{s}</span>
                })}
            </div>

            <Suspense fallback=|| view! { <div class="text-text-secondary text-sm">"loading…"</div> }>
                {move || settings.get().map(|res| match res {
                    Ok(rows) => view! { <KvList rows=rows /> }.into_any(),
                    Err(e) => view! {
                        <div class="text-accent-danger text-sm">{format!("error: {e}")}</div>
                    }.into_any(),
                })}
            </Suspense>

            <div class="rounded border border-border-primary p-3 bg-surface-secondary">
                <h3 class="text-xs uppercase tracking-wider text-text-secondary mb-2">
                    {format!("Add / update setting under '{prefix}'")}
                </h3>
                <div class="grid grid-cols-12 gap-2 items-center">
                    <input
                        class="col-span-4 px-2 py-1 bg-surface-tertiary border border-border-primary rounded text-text-primary text-sm font-mono"
                        placeholder=format!("{prefix}your.key")
                        prop:value=move || new_key.get()
                        on:input=move |ev| set_new_key.set(event_target_value(&ev))
                    />
                    <input
                        class="col-span-6 px-2 py-1 bg-surface-tertiary border border-border-primary rounded text-text-primary text-sm font-mono"
                        placeholder="value (raw → JSON parsed if possible, otherwise string)"
                        prop:value=move || new_val.get()
                        on:input=move |ev| set_new_val.set(event_target_value(&ev))
                    />
                    <label class="col-span-1 flex items-center text-xs text-text-secondary gap-1">
                        <input
                            type="checkbox"
                            prop:checked=move || new_secret.get()
                            on:change=move |ev| set_new_secret.set(event_target_checked(&ev))
                        />
                        "Secret"
                    </label>
                    <button
                        on:click=on_save_new
                        class="col-span-1 px-2 py-1 bg-accent-amber text-surface-primary font-medium rounded text-sm"
                    >
                        "Save"
                    </button>
                </div>
                <p class="text-[10px] text-text-muted mt-2">
                    "Secret values must be vault:// references (e.g. `vault://settings/anthropic_key`). The raw value field is JSON-parsed when possible — strings stay strings."
                </p>
            </div>
        </div>
    }
}

#[component]
fn KvList(rows: Vec<SettingRow>) -> impl IntoView {
    if rows.is_empty() {
        return view! {
            <div class="text-text-secondary text-sm py-4 text-center border border-border-primary rounded">
                "No settings configured for this section yet."
            </div>
        }.into_any();
    }
    view! {
        <table class="w-full text-sm">
            <thead>
                <tr class="text-text-secondary text-left">
                    <th class="py-1">"Key"</th>
                    <th class="py-1">"Value"</th>
                    <th class="py-1">"Secret"</th>
                    <th class="py-1">"Updated"</th>
                </tr>
            </thead>
            <tbody>
                {rows.into_iter().map(|r| {
                    let val_display = if r.is_secret {
                        "•••••• (vault ref)".to_string()
                    } else {
                        match &r.value {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        }
                    };
                    view! {
                        <tr class="border-t border-border-primary">
                            <td class="py-2 font-mono text-xs text-text-primary">{r.key}</td>
                            <td class="py-2 font-mono text-xs text-text-secondary">{val_display}</td>
                            <td class="py-2 text-xs">
                                {if r.is_secret { "yes" } else { "no" }}
                            </td>
                            <td class="py-2 font-mono text-[10px] text-text-muted">{r.updated_at}</td>
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>
    }.into_any()
}

// ---- System tab -------------------------------------------------------------

#[component]
fn SystemTab() -> impl IntoView {
    let info = Resource::new(|| (), |_| get_system_info());
    view! {
        <div class="space-y-4 max-w-3xl">
            <h2 class="text-lg font-semibold text-text-primary">"System"</h2>
            <Suspense fallback=|| view! { <div class="text-text-secondary text-sm">"loading…"</div> }>
                {move || info.get().map(|res| match res {
                    Ok(i) => view! { <SystemInfoView info=i /> }.into_any(),
                    Err(e) => view! {
                        <div class="text-accent-danger text-sm">{format!("error: {e}")}</div>
                    }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn SystemInfoView(info: SystemInfo) -> impl IntoView {
    view! {
        <div class="space-y-4">
            <div class="grid grid-cols-2 gap-x-6 gap-y-2 text-sm">
                <Field label="Version" value=info.version />
                <Field label="Commit" value=info.commit_sha />
                <Field label="Profile" value=info.build_profile />
                <Field label="Host triple" value=info.host_triple />
                <Field label="Tenant name" value=info.tenant_name />
                <Field
                    label="Kill switch"
                    value=if info.kill_switch_engaged {
                        format!("ENGAGED · {}", info.kill_switch_reason.unwrap_or_default())
                    } else {
                        "disengaged".into()
                    }
                />
            </div>
            <div>
                <h3 class="text-xs uppercase tracking-wider text-text-secondary mb-1">
                    "Backends"
                </h3>
                <table class="w-full text-sm">
                    <thead>
                        <tr class="text-text-secondary text-left">
                            <th class="py-1">"Name"</th>
                            <th class="py-1">"URL"</th>
                            <th class="py-1">"Status"</th>
                            <th class="py-1">"Detail"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {info.backends.into_iter().map(|b| view! {
                            <tr class="border-t border-border-primary">
                                <td class="py-2 font-mono text-xs text-text-primary">{b.name}</td>
                                <td class="py-2 font-mono text-xs text-text-secondary">{b.url}</td>
                                <td class=move || format!(
                                    "py-2 text-xs font-mono {}",
                                    if b.reachable { "text-accent-amber" } else { "text-accent-danger" }
                                )>
                                    {if b.reachable { "ok" } else { "unreachable" }}
                                </td>
                                <td class="py-2 text-xs text-text-muted">{b.detail.unwrap_or_default()}</td>
                            </tr>
                        }).collect_view()}
                    </tbody>
                </table>
            </div>
        </div>
    }
}

#[component]
fn Field(label: &'static str, value: String) -> impl IntoView {
    view! {
        <>
            <div class="text-text-secondary">{label}</div>
            <div class="font-mono text-text-primary">{value}</div>
        </>
    }
}

// ---- Update tab -------------------------------------------------------------

#[component]
fn UpdateTab() -> impl IntoView {
    let state = Resource::new(|| (), |_| get_update_state());
    let (status, set_status) = signal::<Option<String>>(None);

    let check_action = Action::new(move |_: &()| async move {
        match check_for_update().await {
            Ok(_) => Some("checked — see below".to_string()),
            Err(e) => Some(format!("check failed: {e}")),
        }
    });

    let apply_action = Action::new(move |_: &()| async move {
        match apply_update().await {
            Ok(_) => Some("update flag written — watch systemd update.service log".to_string()),
            Err(e) => Some(format!("apply failed: {e}")),
        }
    });

    let cancel_action = Action::new(move |_: &()| async move {
        match cancel_update().await {
            Ok(_) => Some("update cancelled — flag removed".to_string()),
            Err(e) => Some(format!("cancel failed: {e}")),
        }
    });

    Effect::new(move |_| {
        for a in [&check_action, &apply_action, &cancel_action] {
            if let Some(m) = a.value().get().flatten() {
                set_status.set(Some(m));
                state.refetch();
            }
        }
    });

    let channel_action = Action::new(move |c: &String| {
        let c = c.clone();
        async move {
            match set_setting(
                "update.channel".into(),
                serde_json::Value::String(c.clone()),
                false,
            )
            .await
            {
                Ok(_) => Some(format!("channel set to {c}")),
                Err(e) => Some(format!("channel save failed: {e}")),
            }
        }
    });

    Effect::new(move |_| {
        if let Some(m) = channel_action.value().get().flatten() {
            set_status.set(Some(m));
            state.refetch();
        }
    });

    view! {
        <div class="space-y-4 max-w-3xl">
            <div class="flex items-center justify-between">
                <h2 class="text-lg font-semibold text-text-primary">"Update"</h2>
                {move || status.get().map(|s| view! {
                    <span class="text-xs font-mono text-text-secondary">{s}</span>
                })}
            </div>
            <Suspense fallback=|| view! { <div class="text-text-secondary text-sm">"loading…"</div> }>
                {move || state.get().map(|res| match res {
                    Ok(s) => {
                        let s_for_channel = s.clone();
                        view! { <UpdateView state=s on_check=move |_| { check_action.dispatch(()); } on_apply=move |_| { apply_action.dispatch(()); } on_cancel=move |_| { cancel_action.dispatch(()); } on_channel=move |c| { channel_action.dispatch(c); } /> }.into_any()
                    }
                    Err(e) => view! {
                        <div class="text-accent-danger text-sm">{format!("error: {e}")}</div>
                    }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn UpdateView(
    state: UpdateState,
    #[prop(into)] on_check: Callback<()>,
    #[prop(into)] on_apply: Callback<()>,
    #[prop(into)] on_cancel: Callback<()>,
    #[prop(into)] on_channel: Callback<String>,
) -> impl IntoView {
    // Two channels mirror the dev/staging/prod git workflow we locked:
    //   stable → GitHub releases (production-promoted via `just promote`)
    //   beta   → GitLab releases (staging, default `git push` target)
    let channels = ["stable", "beta"];
    let current_channel = state.channel.clone();
    let update_available = state
        .latest_tag
        .as_deref()
        .map(|t| !t.contains(&state.current_version))
        .unwrap_or(false);

    view! {
        <div class="space-y-4">
            <div class="grid grid-cols-2 gap-x-6 gap-y-2 text-sm">
                <Field label="Current version" value=state.current_version.clone() />
                <Field label="Current commit" value=state.current_commit.clone() />
                <Field
                    label="Latest available"
                    value=state.latest_tag.clone().unwrap_or_else(|| "(no check yet)".into())
                />
                <Field
                    label="Last check"
                    value=state.last_check_at.clone().unwrap_or_else(|| "never".into())
                />
                <Field label="Update flag" value=state.update_flag_path.clone() />
                <Field
                    label="Pending"
                    value=if state.update_pending { "yes — apply requested".into() } else { "no".into() }
                />
            </div>

            <div class="rounded border border-border-primary p-3 bg-surface-secondary">
                <h3 class="text-xs uppercase tracking-wider text-text-secondary mb-2">
                    "Release channel"
                </h3>
                <div class="flex gap-2">
                    {channels.iter().map(|ch| {
                        let label = *ch;
                        let label_for_class = label;
                        let label_for_click = label;
                        let is_current = current_channel == label;
                        view! {
                            <button
                                on:click=move |_| on_channel.run(label_for_click.to_string())
                                class=move || format!(
                                    "px-3 py-1 rounded text-sm font-mono {}",
                                    if is_current {
                                        "bg-accent-amber text-surface-primary"
                                    } else {
                                        "bg-surface-tertiary text-text-primary hover:bg-surface-secondary border border-border-primary"
                                    }
                                )
                            >
                                {label_for_class}
                            </button>
                        }
                    }).collect_view()}
                </div>
                <p class="text-[10px] text-text-muted mt-2">
                    "stable → GitHub releases (production-promoted). beta → GitLab releases (staging — default push target)."
                </p>
            </div>

            <div class="flex gap-2">
                <button
                    on:click=move |_| on_check.run(())
                    class="px-3 py-1.5 bg-surface-tertiary text-text-primary border border-border-primary rounded text-sm"
                >
                    "Check for update"
                </button>
                <button
                    on:click=move |_| on_apply.run(())
                    class="px-3 py-1.5 bg-accent-amber text-surface-primary font-medium rounded text-sm disabled:opacity-50"
                    prop:disabled=!update_available && !state.update_pending
                >
                    "Apply update"
                </button>
                {state.update_pending.then(|| view! {
                    <button
                        on:click=move |_| on_cancel.run(())
                        class="px-3 py-1.5 bg-accent-danger/80 text-text-primary rounded text-sm"
                    >
                        "Cancel pending"
                    </button>
                })}
            </div>

            <div class="rounded border border-border-primary p-3 bg-surface-secondary text-xs text-text-secondary">
                <p class="mb-1 font-semibold text-text-primary">"How apply works"</p>
                <p>
                    "Clicking Apply writes the target tag to the flag file shown above. A systemd path-unit watches the path and triggers "
                    <code class="bg-surface-tertiary px-1 rounded">"daimon-update.service"</code>
                    " which:"
                </p>
                <ol class="list-decimal list-inside mt-1 space-y-0.5">
                    <li>"backs up the current binary"</li>
                    <li>"downloads + extracts the release asset matching the host triple"</li>
                    <li>"restarts daimon-agent@* / daimon-app via systemctl"</li>
                    <li>"on boot failure within 60s, restores the backup binary"</li>
                </ol>
                <p class="mt-1">
                    "On macOS dev (no systemd) the flag still gets written; the operator runs the equivalent update hook by hand. See "
                    <code class="bg-surface-tertiary px-1 rounded">"deploy/systemd/daimon-update.service"</code>
                    "."
                </p>
            </div>
        </div>
    }
}
