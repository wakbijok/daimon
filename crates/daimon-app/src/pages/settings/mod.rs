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

use crate::admin_gateways::{
    add_gateway_binding, delete_gateway_binding, list_gateway_bindings, GatewayBindingDto,
};
use crate::admin_settings::{
    apply_update, cancel_update, check_for_update, get_config_reference, get_system_info,
    get_update_state, list_settings, set_setting, SettingRow, SystemInfo, UpdateState,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Identity,
    Iam,
    Targets,
    Credentials,
    Connections,
    Llm,
    Guard,
    Observer,
    Rag,
    Vault,
    Channels,
    System,
    Update,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Identity => "Identity",
            Tab::Iam => "IAM",
            Tab::Targets => "Targets",
            Tab::Credentials => "Credentials",
            Tab::Connections => "Connections",
            Tab::Llm => "LLM Providers",
            Tab::Guard => "Guard",
            Tab::Observer => "Observer",
            Tab::Rag => "RAG & Memory",
            Tab::Vault => "Vault & KMS",
            Tab::Channels => "Channels",
            Tab::System => "System",
            Tab::Update => "Update",
        }
    }
    fn prefix(self) -> &'static str {
        match self {
            Tab::Identity => "identity.",
            Tab::Iam => "iam.",
            Tab::Targets => "targets.",
            Tab::Credentials => "credentials.",
            Tab::Connections => "connections.",
            Tab::Llm => "llm.",
            Tab::Guard => "guard.",
            Tab::Observer => "observer.",
            Tab::Rag => "rag.",
            Tab::Vault => "vault.",
            Tab::Channels => "channels.",
            Tab::System => "",
            Tab::Update => "update.",
        }
    }

    /// P6-6 (FR-CFG-01/03/10): a tab whose keys the runtime does NOT consume in
    /// this build renders READ-ONLY — the editor is hidden and a banner explains
    /// why — so a field can never claim to change behaviour that nothing reads.
    /// `Connections` is bootstrap/env-sourced (FR-CFG-03); `Vault & KMS` is
    /// KMS-dead in revival scope (FR-CFG-10).
    fn read_only(self) -> bool {
        matches!(self, Tab::Connections | Tab::Vault)
    }

    /// Why a read-only tab is read-only (shown in the banner).
    fn read_only_reason(self) -> &'static str {
        match self {
            Tab::Connections => "Backend connection settings are sourced from the environment / \
                systemd unit at boot (DAIMON_PG_URL, master key, DAIMON_DATA_DIR) and cannot be \
                edited here (FR-CFG-03).",
            Tab::Vault => "KMS / DEK-rotation wiring is out of revival scope; these values are \
                shown for reference only (FR-CFG-10).",
            _ => "",
        }
    }
}

const TAB_ORDER: [Tab; 13] = [
    Tab::Identity,
    Tab::Iam,
    Tab::Targets,
    Tab::Credentials,
    Tab::Connections,
    Tab::Llm,
    Tab::Guard,
    Tab::Observer,
    Tab::Rag,
    Tab::Vault,
    Tab::Channels,
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
                    Tab::Channels => view! { <ChannelsTab /> }.into_any(),
                    // P6-7/P6-8: the Targets/Connectors + IAM domains reuse the
                    // existing admin surfaces (inventory targets + IAM users are
                    // already runtime-consumed) — no new consumption path.
                    Tab::Targets => view! { <crate::pages::admin::targets::AdminTargets /> }.into_any(),
                    Tab::Iam => view! { <crate::pages::admin::iam::AdminIam /> }.into_any(),
                    // UI-1: Credentials config absorbed from /admin/credentials.
                    Tab::Credentials => view! { <crate::pages::admin::credentials::AdminCredentials /> }.into_any(),
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

            {if tab.read_only() {
                view! {
                    <div class="rounded border border-border-primary bg-surface-secondary p-3 text-xs text-text-secondary">
                        <span class="font-semibold text-accent-amber">"Read-only. "</span>
                        {tab.read_only_reason()}
                    </div>
                }.into_any()
            } else {
                view! {}.into_any()
            }}

            <Suspense fallback=|| view! { <div class="text-text-secondary text-sm">"loading…"</div> }>
                {move || settings.get().map(|res| match res {
                    Ok(rows) => view! { <KvList rows=rows /> }.into_any(),
                    Err(e) => view! {
                        <div class="text-accent-danger text-sm">{format!("error: {e}")}</div>
                    }.into_any(),
                })}
            </Suspense>

            // P6-6: consumed-key reference — an editable tab shows exactly which
            // keys the runtime reads, so no field is mistaken for live when it is
            // not. Read-only tabs skip the editor entirely.
            {if tab.read_only() {
                view! {}.into_any()
            } else {
                let consumed = crate::config_keys::consumed_keys_under(prefix);
                view! {
                    {if consumed.is_empty() { view! {}.into_any() } else {
                        view! {
                            <div class="rounded border border-border-primary p-3 bg-surface-secondary/40">
                                <h3 class="text-xs uppercase tracking-wider text-text-secondary mb-2">
                                    "Consumed by the runtime"
                                </h3>
                                <ul class="space-y-1">
                                    {consumed.into_iter().map(|(k, desc)| view! {
                                        <li class="text-xs">
                                            <code class="text-accent-amber">{k}</code>
                                            <span class="text-text-muted">" — "{desc}</span>
                                        </li>
                                    }).collect_view()}
                                </ul>
                            </div>
                        }.into_any()
                    }}

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
                            "Secret values (checkbox) are intercepted server-side: the plaintext is stored in the vault and only a vault:// reference is persisted. The raw value field is JSON-parsed when possible — strings stay strings."
                        </p>
                    </div>
                }.into_any()
            }}
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
    // P6-13 (FR-CFG-04): the code-derived config reference, on demand.
    let reference = Resource::new(|| (), |_| get_config_reference());
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

            <details class="rounded border border-border-primary bg-surface-secondary p-3">
                <summary class="text-sm text-text-primary cursor-pointer">
                    "Configuration reference (code-derived)"
                </summary>
                <Suspense fallback=|| view! { <div class="text-text-secondary text-sm mt-2">"loading…"</div> }>
                    {move || reference.get().map(|res| match res {
                        Ok(md) => view! {
                            <pre class="text-[11px] font-mono text-text-secondary whitespace-pre-wrap mt-2 overflow-x-auto">{md}</pre>
                        }.into_any(),
                        Err(e) => view! {
                            <div class="text-accent-danger text-sm">{format!("error: {e}")}</div>
                        }.into_any(),
                    })}
                </Suspense>
            </details>
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

// ---- Channels tab (P4, FR-GW-16/17) ----------------------------------------

/// The messaging-gateway configuration surface. Channel config (enable toggles,
/// homeserver URLs, credential-name references) lives in `app_config` under
/// `channels.*` via the generic KV editor; bot tokens + signing secrets are held
/// in the Vault (created under **Vault & KMS** → referenced here by credential
/// name), never inline (FR-GW-17). Below the config is the identity-enrolment
/// table: which chat-platform handle maps to which IAM user (FR-GW-08).
#[component]
fn ChannelsTab() -> impl IntoView {
    view! {
        <div class="space-y-6">
            <div class="rounded border border-border-primary p-3 bg-surface-secondary text-xs text-text-secondary max-w-3xl">
                <p class="mb-1 font-semibold text-text-primary">"Messaging gateways"</p>
                <p>
                    "Reach daimon from Telegram or Matrix. A channel message runs the SAME chat + tool path a browser turn takes — bound to a real IAM identity, gated by the same policy + approval. A channel is a front door, never a bypass."
                </p>
                <ul class="list-disc list-inside mt-2 space-y-0.5">
                    <li>
                        <span class="font-mono">"channels.telegram.enabled"</span>
                        " = true, "
                        <span class="font-mono">"channels.telegram.mode"</span>
                        " = poll (default — getUpdates, no ingress) or webhook, "
                        <span class="font-mono">"channels.telegram.bot_token_cred"</span>
                        " = <vault credential name>. Webhook mode also needs "
                        <span class="font-mono">"channels.telegram.webhook_secret_cred"</span>
                        "."
                    </li>
                    <li>
                        <span class="font-mono">"channels.matrix.enabled"</span>
                        " = true, "
                        <span class="font-mono">"channels.matrix.homeserver"</span>
                        " = https://matrix.example.org, "
                        <span class="font-mono">"channels.matrix.access_token_cred"</span>
                        " = <vault credential name>"
                    </li>
                </ul>
                <p class="mt-2">
                    "Bot tokens + secrets are stored as ApiToken credentials under "
                    <span class="font-mono">"Vault & KMS"</span>
                    " and referenced here by name — the token itself never lives in app_config or a log (FR-GW-17). Changes take effect on the next daimon restart."
                </p>
            </div>

            <KvTab tab=Tab::Channels />

            <GatewayEnrolment />
        </div>
    }
}

/// The `gateway_identities` enrolment table — admin add/revoke of
/// chat-handle -> IAM-user bindings (FR-GW-08). A handle with no row here is
/// refused fail-closed at inbound time; enrolment is the authorization.
#[component]
fn GatewayEnrolment() -> impl IntoView {
    let bindings = Resource::new(|| (), |_| list_gateway_bindings());
    let (status, set_status) = signal::<Option<String>>(None);
    let (new_channel, set_new_channel) = signal("telegram".to_string());
    let (new_handle, set_new_handle) = signal(String::new());
    let (new_user, set_new_user) = signal(String::new());

    let add_action = Action::new(move |args: &(String, String, String)| {
        let (c, h, u) = args.clone();
        async move {
            match add_gateway_binding(c, h, u).await {
                Ok(_) => Some("binding added".to_string()),
                Err(e) => Some(format!("error: {e}")),
            }
        }
    });
    let del_action = Action::new(move |id: &uuid::Uuid| {
        let id = *id;
        async move {
            match delete_gateway_binding(id).await {
                Ok(_) => Some("binding revoked".to_string()),
                Err(e) => Some(format!("error: {e}")),
            }
        }
    });

    Effect::new(move |_| {
        if let Some(m) = add_action.value().get().flatten() {
            set_status.set(Some(m));
            bindings.refetch();
        }
    });
    Effect::new(move |_| {
        if let Some(m) = del_action.value().get().flatten() {
            set_status.set(Some(m));
            bindings.refetch();
        }
    });

    let on_add = move |_| {
        let (c, h, u) = (
            new_channel.get().trim().to_lowercase(),
            new_handle.get().trim().to_string(),
            new_user.get().trim().to_string(),
        );
        if h.is_empty() || u.is_empty() {
            set_status.set(Some("platform handle and username are required".into()));
            return;
        }
        add_action.dispatch((c, h, u));
        set_new_handle.set(String::new());
        set_new_user.set(String::new());
    };

    view! {
        <div class="space-y-3 max-w-3xl">
            <div class="flex items-center justify-between">
                <h2 class="text-lg font-semibold text-text-primary">"Identity enrolment"</h2>
                {move || status.get().map(|s| view! {
                    <span class="text-xs font-mono text-text-secondary">{s}</span>
                })}
            </div>

            <Suspense fallback=|| view! { <div class="text-text-secondary text-sm">"loading…"</div> }>
                {move || bindings.get().map(|res| match res {
                    Ok(rows) => view! { <BindingsList rows=rows on_delete=Callback::new(move |id| { del_action.dispatch(id); }) /> }.into_any(),
                    Err(e) => view! {
                        <div class="text-accent-danger text-sm">{format!("error: {e}")}</div>
                    }.into_any(),
                })}
            </Suspense>

            <div class="rounded border border-border-primary p-3 bg-surface-secondary">
                <h3 class="text-xs uppercase tracking-wider text-text-secondary mb-2">
                    "Enrol a handle -> IAM user"
                </h3>
                <div class="grid grid-cols-12 gap-2 items-center">
                    <select
                        class="col-span-3 px-2 py-1 bg-surface-tertiary border border-border-primary rounded text-text-primary text-sm"
                        on:change=move |ev| set_new_channel.set(event_target_value(&ev))
                    >
                        <option value="telegram">"telegram"</option>
                        <option value="matrix">"matrix"</option>
                    </select>
                    <input
                        class="col-span-4 px-2 py-1 bg-surface-tertiary border border-border-primary rounded text-text-primary text-sm font-mono"
                        placeholder="platform handle (tg user id / @mxid)"
                        prop:value=move || new_handle.get()
                        on:input=move |ev| set_new_handle.set(event_target_value(&ev))
                    />
                    <input
                        class="col-span-3 px-2 py-1 bg-surface-tertiary border border-border-primary rounded text-text-primary text-sm font-mono"
                        placeholder="daimon username"
                        prop:value=move || new_user.get()
                        on:input=move |ev| set_new_user.set(event_target_value(&ev))
                    />
                    <button
                        on:click=on_add
                        class="col-span-2 px-2 py-1 bg-accent-amber text-surface-primary font-medium rounded text-sm"
                    >
                        "Enrol"
                    </button>
                </div>
                <p class="text-[10px] text-text-muted mt-2">
                    "The handle is the sender's stable platform id — a Telegram numeric user id or a Matrix MXID (@user:server). An unmapped handle is refused fail-closed; no capability runs for it."
                </p>
            </div>
        </div>
    }
}

#[component]
fn BindingsList(
    rows: Vec<GatewayBindingDto>,
    #[prop(into)] on_delete: Callback<uuid::Uuid>,
) -> impl IntoView {
    if rows.is_empty() {
        return view! {
            <div class="text-text-secondary text-sm py-4 text-center border border-border-primary rounded">
                "No handles enrolled yet. Add one below to let a chat-platform user reach daimon."
            </div>
        }.into_any();
    }
    view! {
        <table class="w-full text-sm">
            <thead>
                <tr class="text-text-secondary text-left">
                    <th class="py-1">"Channel"</th>
                    <th class="py-1">"Platform handle"</th>
                    <th class="py-1">"IAM user"</th>
                    <th class="py-1">"Enrolled"</th>
                    <th class="py-1"></th>
                </tr>
            </thead>
            <tbody>
                {rows.into_iter().map(|r| {
                    let id = r.id;
                    view! {
                        <tr class="border-t border-border-primary">
                            <td class="py-2 font-mono text-xs text-text-primary">{r.channel}</td>
                            <td class="py-2 font-mono text-xs text-text-secondary">{r.platform_handle}</td>
                            <td class="py-2 font-mono text-xs text-text-primary">{r.username}</td>
                            <td class="py-2 font-mono text-[10px] text-text-muted">{r.enrolled_at}</td>
                            <td class="py-2 text-right">
                                <button
                                    on:click=move |_| on_delete.run(id)
                                    class="px-2 py-0.5 bg-accent-danger/80 text-text-primary rounded text-xs"
                                >
                                    "Revoke"
                                </button>
                            </td>
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>
    }.into_any()
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
