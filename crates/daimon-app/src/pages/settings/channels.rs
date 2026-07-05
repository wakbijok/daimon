//! UI-8 — Channels settings, Hermes Gateway-style (console v2 review round).
//!
//! Wak's reference: Hermes's gateway settings — a card per messaging platform
//! with a state badge, an enable toggle, typed per-platform fields, and a
//! "restart to apply" indicator. Ours carries the same shape for the two
//! channels the runtime actually wires at boot (`main.rs` gateway section):
//! Telegram (poll/webhook) and Matrix (/sync poller). Every field writes a
//! runtime-consumed `channels.*` key through `set_setting` — no free-form KV.
//!
//! Credential fields hold vault credential NAMES (created under Vault & KMS /
//! Credentials), never token plaintext (FR-GW-17). Identity enrolment (the
//! fail-closed platform-handle → IAM-user binding, our stronger version of
//! Hermes's `allow_from` list) keeps its table below the cards.

use leptos::prelude::*;
use serde_json::Value as Json;

use crate::admin_gateways::{
    GatewayBindingDto, add_gateway_binding, delete_gateway_binding, list_gateway_bindings,
};
use crate::admin_settings::{SettingRow, list_settings, set_setting};

fn row_string(rows: &[SettingRow], key: &str) -> String {
    rows.iter()
        .find(|r| r.key == key)
        .map(|r| match &r.value {
            Json::String(s) => s.clone(),
            Json::Bool(b) => b.to_string(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

fn row_bool(rows: &[SettingRow], key: &str) -> bool {
    row_string(rows, key) == "true"
}

#[component]
pub fn ChannelsTab() -> impl IntoView {
    let settings = LocalResource::new(move || list_settings("channels.".to_string()));
    let (status, set_status) = signal::<Option<String>>(None);

    view! {
        <div class="max-w-4xl space-y-6">
            <div class="flex items-start justify-between gap-4">
                <div>
                    <h2 class="text-lg font-semibold text-text-primary">"Messaging channels"</h2>
                    <p class="text-[12px] text-text-muted mt-0.5 max-w-xl leading-snug">
                        "Reach daimon from a chat platform. A channel message runs the SAME chat + \
                         tool path a browser turn takes — bound to a real IAM identity, gated by the \
                         same policy and approvals. A channel is a front door, never a bypass. \
                         Changes apply on the next daimon restart."
                    </p>
                </div>
                {move || status.get().map(|s| view! {
                    <span class="text-xs font-mono text-text-secondary whitespace-nowrap pt-1">{s}</span>
                })}
            </div>

            <Suspense fallback=|| view! {
                <div class="text-text-muted text-sm py-8 text-center">"loading channels…"</div>
            }>
                {move || settings.get().map(|res| match res {
                    Err(e) => view! {
                        <div class="text-accent-danger text-sm">{format!("error: {e}")}</div>
                    }.into_any(),
                    Ok(rows) => {
                        let rows_tg = rows.clone();
                        let rows_mx = rows;
                        view! {
                            <div class="space-y-4">
                                <TelegramCard rows=rows_tg settings=settings set_status=set_status />
                                <MatrixCard rows=rows_mx settings=settings set_status=set_status />
                            </div>
                        }.into_any()
                    }
                })}
            </Suspense>

            <AlertRoutingAdvanced />

            <GatewayEnrolment />
        </div>
    }
}

/// Shared card chrome: glyph tile + name + state badge + toggle row.
#[component]
fn ChannelShell(
    glyph: &'static str,
    accent: &'static str,
    name: &'static str,
    tagline: &'static str,
    enabled: bool,
    ready: bool,
    missing: &'static str,
    on_toggle: Callback<bool>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class=format!(
            "rounded-lg border p-5 space-y-4 {}",
            if enabled { "border-accent-amber/40 bg-accent-amber/[0.03]" } else { "border-border-primary bg-surface-secondary" }
        )>
            <div class="flex items-start gap-3">
                <div class=format!(
                    "shrink-0 w-11 h-11 rounded-md border flex items-center justify-center font-semibold text-sm bg-surface-tertiary {accent}"
                )>
                    {glyph}
                </div>
                <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2 flex-wrap">
                        <span class="text-sm font-semibold text-text-primary">{name}</span>
                        {if !enabled {
                            view! {
                                <span class="text-[9.5px] font-semibold uppercase tracking-wider px-1.5 py-0.5 rounded bg-surface-tertiary text-text-muted">
                                    "Disabled"
                                </span>
                            }.into_any()
                        } else if ready {
                            view! {
                                <span class="text-[9.5px] font-semibold uppercase tracking-wider px-1.5 py-0.5 rounded bg-accent-green/20 text-accent-green">
                                    "Enabled"
                                </span>
                            }.into_any()
                        } else {
                            view! {
                                <span class="text-[9.5px] font-semibold uppercase tracking-wider px-1.5 py-0.5 rounded bg-accent-danger/20 text-accent-danger">
                                    {format!("Enabled — missing {missing}")}
                                </span>
                            }.into_any()
                        }}
                    </div>
                    <p class="text-[11.5px] text-text-muted mt-1 leading-snug">{tagline}</p>
                </div>
                <label class="inline-flex items-center gap-2 cursor-pointer shrink-0 pt-1">
                    <input type="checkbox"
                        prop:checked=enabled
                        on:change=move |ev| on_toggle.run(event_target_checked(&ev))
                        class="w-4 h-4 accent-accent-amber" />
                    <span class="text-xs text-text-secondary">{if enabled { "On" } else { "Off" }}</span>
                </label>
            </div>
            {children()}
        </div>
    }
}

/// One labelled field row inside a channel card.
#[component]
fn ChannelField(label: &'static str, help: &'static str, children: Children) -> impl IntoView {
    view! {
        <div class="grid gap-4" style="grid-template-columns:200px 1fr">
            <div class="pt-1.5">
                <div class="text-[13px] font-semibold text-text-primary">{label}</div>
                <div class="text-[11px] text-text-muted mt-0.5 leading-snug">{help}</div>
            </div>
            <div class="max-w-md">{children()}</div>
        </div>
    }
}

const INPUT_CLS: &str = "w-full px-3 py-1.5 bg-surface-tertiary border border-border-primary \
     rounded-md text-sm text-text-primary font-mono focus:outline-none focus:border-accent-amber";

// ---- Telegram ----------------------------------------------------------------

#[component]
fn TelegramCard(
    rows: Vec<SettingRow>,
    settings: LocalResource<Result<Vec<SettingRow>, ServerFnError>>,
    set_status: WriteSignal<Option<String>>,
) -> impl IntoView {
    let enabled = row_bool(&rows, "channels.telegram.enabled");
    let seed_mode = {
        let m = row_string(&rows, "channels.telegram.mode");
        if m.is_empty() { "poll".to_string() } else { m }
    };
    let seed_token_cred = row_string(&rows, "channels.telegram.bot_token_cred");
    let seed_secret_cred = row_string(&rows, "channels.telegram.webhook_secret_cred");
    let ready = !seed_token_cred.is_empty();

    let mode = RwSignal::new(seed_mode);
    let token_cred = RwSignal::new(seed_token_cred);
    let secret_cred = RwSignal::new(seed_secret_cred);

    let toggle = Action::new(move |on: &bool| {
        let on = *on;
        async move {
            match set_setting("channels.telegram.enabled".into(), Json::Bool(on), false).await {
                Ok(_) => format!(
                    "telegram {} — applies on next restart",
                    if on { "enabled" } else { "disabled" }
                ),
                Err(e) => format!("error: {e}"),
            }
        }
    });
    let save = Action::new(move |_: &()| async move {
        let writes = [
            ("channels.telegram.mode", mode.get_untracked()),
            (
                "channels.telegram.bot_token_cred",
                token_cred.get_untracked(),
            ),
            (
                "channels.telegram.webhook_secret_cred",
                secret_cred.get_untracked(),
            ),
        ];
        for (k, v) in writes {
            if v.trim().is_empty() {
                continue; // blank = unchanged
            }
            if let Err(e) = set_setting(k.to_string(), Json::String(v), false).await {
                return format!("error: {e}");
            }
        }
        "telegram saved — applies on next restart ✓".to_string()
    });
    Effect::new(move |_| {
        for msg in [toggle.value().get(), save.value().get()]
            .into_iter()
            .flatten()
        {
            set_status.set(Some(msg));
            settings.refetch();
        }
    });

    view! {
        <ChannelShell
            glyph="TG" accent="text-sky-400 border-sky-400/40"
            name="Telegram" tagline="Bot API — long-poll (no ingress needed) or webhook."
            enabled=enabled ready=ready missing="bot token credential"
            on_toggle=Callback::new(move |on| { toggle.dispatch(on); })
        >
            <div class="space-y-4">
                <ChannelField label="Ingress mode" help="poll = getUpdates from inside your network (default). webhook = Telegram calls your public endpoint.">
                    <div class="inline-flex border border-border-primary rounded-md overflow-hidden">
                        {["poll", "webhook"].into_iter().map(|opt| view! {
                            <button
                                on:click=move |_| mode.set(opt.to_string())
                                class=move || format!(
                                    "px-3 py-1.5 text-xs font-mono transition-colors {}",
                                    if mode.get() == opt { "bg-accent-amber/15 text-accent-amber" } else { "text-text-muted hover:text-text-primary" }
                                )
                            >{opt}</button>
                        }).collect_view()}
                    </div>
                </ChannelField>
                <ChannelField label="Bot token credential" help="Name of the ApiToken credential holding the bot token (create it under Credentials). The token itself never lives here.">
                    <input type="text" placeholder="telegram-bot-token" class=INPUT_CLS
                        prop:value=move || token_cred.get()
                        on:input=move |ev| token_cred.set(event_target_value(&ev)) />
                </ChannelField>
                {move || (mode.get() == "webhook").then(|| view! {
                    <ChannelField label="Webhook secret credential" help="Credential holding the webhook signing secret — required in webhook mode; inbound calls failing the signature are dropped.">
                        <input type="text" placeholder="telegram-webhook-secret" class=INPUT_CLS
                            prop:value=move || secret_cred.get()
                            on:input=move |ev| secret_cred.set(event_target_value(&ev)) />
                    </ChannelField>
                })}
                <div class="flex justify-end">
                    <button
                        on:click=move |_| { save.dispatch(()); }
                        class="px-4 py-1.5 bg-accent-amber text-surface-primary font-medium rounded-md text-sm"
                    >"Save Telegram"</button>
                </div>
            </div>
        </ChannelShell>
    }
}

// ---- Matrix -------------------------------------------------------------------

#[component]
fn MatrixCard(
    rows: Vec<SettingRow>,
    settings: LocalResource<Result<Vec<SettingRow>, ServerFnError>>,
    set_status: WriteSignal<Option<String>>,
) -> impl IntoView {
    let enabled = row_bool(&rows, "channels.matrix.enabled");
    let seed_homeserver = row_string(&rows, "channels.matrix.homeserver");
    let seed_token_cred = row_string(&rows, "channels.matrix.access_token_cred");
    let ready = !seed_homeserver.is_empty() && !seed_token_cred.is_empty();

    let homeserver = RwSignal::new(seed_homeserver);
    let token_cred = RwSignal::new(seed_token_cred);

    let toggle = Action::new(move |on: &bool| {
        let on = *on;
        async move {
            match set_setting("channels.matrix.enabled".into(), Json::Bool(on), false).await {
                Ok(_) => format!(
                    "matrix {} — applies on next restart",
                    if on { "enabled" } else { "disabled" }
                ),
                Err(e) => format!("error: {e}"),
            }
        }
    });
    let save = Action::new(move |_: &()| async move {
        let writes = [
            ("channels.matrix.homeserver", homeserver.get_untracked()),
            (
                "channels.matrix.access_token_cred",
                token_cred.get_untracked(),
            ),
        ];
        for (k, v) in writes {
            if v.trim().is_empty() {
                continue;
            }
            if let Err(e) = set_setting(k.to_string(), Json::String(v), false).await {
                return format!("error: {e}");
            }
        }
        "matrix saved — applies on next restart ✓".to_string()
    });
    Effect::new(move |_| {
        for msg in [toggle.value().get(), save.value().get()]
            .into_iter()
            .flatten()
        {
            set_status.set(Some(msg));
            settings.refetch();
        }
    });

    view! {
        <ChannelShell
            glyph="[m]" accent="text-accent-green border-accent-green/40"
            name="Matrix" tagline="Homeserver /sync poller — self-hosted friendly, no ingress needed."
            enabled=enabled ready=ready missing="homeserver / token credential"
            on_toggle=Callback::new(move |on| { toggle.dispatch(on); })
        >
            <div class="space-y-4">
                <ChannelField label="Homeserver" help="Base URL of the Matrix homeserver the bot account lives on.">
                    <input type="text" placeholder="https://matrix.example.org" class=INPUT_CLS
                        prop:value=move || homeserver.get()
                        on:input=move |ev| homeserver.set(event_target_value(&ev)) />
                </ChannelField>
                <ChannelField label="Access token credential" help="Name of the ApiToken credential holding the bot's Matrix access token (create it under Credentials).">
                    <input type="text" placeholder="matrix-access-token" class=INPUT_CLS
                        prop:value=move || token_cred.get()
                        on:input=move |ev| token_cred.set(event_target_value(&ev)) />
                </ChannelField>
                <div class="flex justify-end">
                    <button
                        on:click=move |_| { save.dispatch(()); }
                        class="px-4 py-1.5 bg-accent-amber text-surface-primary font-medium rounded-md text-sm"
                    >"Save Matrix"</button>
                </div>
            </div>
        </ChannelShell>
    }
}

// ---- Alert routing (advanced, rule-shaped keys) --------------------------------

/// The `channels.alerts.<class>.<severity>` routing rules stay raw for now —
/// they are rule-shaped (arbitrary class/severity pairs), so a fixed form can't
/// represent them. Collapsed, with the key shape documented.
#[component]
fn AlertRoutingAdvanced() -> impl IntoView {
    let settings = LocalResource::new(move || list_settings("channels.alerts.".to_string()));
    view! {
        <details class="rounded-lg border border-border-primary bg-surface-secondary/40">
            <summary class="px-4 py-2.5 text-xs text-text-secondary cursor-pointer select-none">
                "Alert routing rules — " <code class="text-accent-amber">"channels.alerts.<class>.<severity>"</code>
                " → recipient (advanced)"
            </summary>
            <div class="px-4 pb-3 space-y-2">
                <p class="text-[11px] text-text-muted leading-snug">
                    "Routes outbound alerts by class + severity to a channel recipient, e.g. "
                    <code class="bg-surface-tertiary px-1 rounded">"channels.alerts.anomaly.critical"</code>
                    " = "
                    <code class="bg-surface-tertiary px-1 rounded">"telegram:<chat_id>"</code>
                    ". Add or edit rules via the System → Advanced raw editor; current rules:"
                </p>
                <Suspense fallback=|| view! { <div class="text-text-muted text-xs py-2">"loading…"</div> }>
                    {move || settings.get().map(|res| match res {
                        Ok(rows) if rows.is_empty() => view! {
                            <div class="text-text-muted text-xs py-2">"no routing rules set — alerts stay in-console only"</div>
                        }.into_any(),
                        Ok(rows) => view! {
                            <table class="w-full text-xs">
                                <tbody>
                                    {rows.into_iter().map(|r| {
                                        let val = match &r.value { Json::String(s) => s.clone(), o => o.to_string() };
                                        view! {
                                            <tr class="border-t border-border-primary/40">
                                                <td class="py-1.5 font-mono text-text-primary pr-4">{r.key}</td>
                                                <td class="py-1.5 font-mono text-text-muted">{val}</td>
                                            </tr>
                                        }
                                    }).collect_view()}
                                </tbody>
                            </table>
                        }.into_any(),
                        Err(e) => view! { <div class="text-accent-danger text-xs py-2">{format!("error: {e}")}</div> }.into_any(),
                    })}
                </Suspense>
            </div>
        </details>
    }
}

// ---- Identity enrolment (moved from settings/mod.rs, unchanged behaviour) ------

/// The `gateway_identities` enrolment table — admin add/revoke of
/// chat-handle -> IAM-user bindings (FR-GW-08). A handle with no row here is
/// refused fail-closed at inbound time; enrolment is the authorization. (Our
/// stronger equivalent of Hermes's `allow_from` allowlist.)
#[component]
fn GatewayEnrolment() -> impl IntoView {
    let bindings = LocalResource::new(move || list_gateway_bindings());
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
        <div class="space-y-3">
            <div class="flex items-center justify-between">
                <div>
                    <h2 class="text-sm font-semibold text-text-primary">"Identity enrolment"</h2>
                    <p class="text-[11px] text-text-muted mt-0.5">
                        "Which chat-platform handle maps to which IAM user. An unmapped handle is refused fail-closed — enrolment IS the authorization."
                    </p>
                </div>
                {move || status.get().map(|s| view! {
                    <span class="text-xs font-mono text-text-secondary">{s}</span>
                })}
            </div>

            <Suspense fallback=|| view! { <div class="text-text-muted text-sm">"loading…"</div> }>
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
                    "The handle is the sender's stable platform id — a Telegram numeric user id or a Matrix MXID (@user:server)."
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
        }
        .into_any();
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
    }
    .into_any()
}
