//! `/admin/audit` — paged read-only audit log viewer (Phase 2b #14).
//!
//! Filter form (actor / action / target_ref substring / result + time range
//! presets + custom datetime-local inputs) drives a `Resource` that fetches
//! the current page from `list_audit_events`. Pagination uses
//! `count_audit_events` for the total. Row click opens a detail modal with
//! full metadata + op_summary + credential_ref + latency + KV metadata.
//!
//! Time wire format: epoch seconds. Browser-side conversion via
//! `js_sys::Date` (datetime-local string → epoch ms → epoch s).

use leptos::prelude::*;

use crate::admin_audit::{
    count_audit_events, list_audit_events, ActionKindDto, AuditEventRow, AuditFilterDto,
    AuditResultDto,
};
use crate::components::modal::Modal;

const PAGE_SIZE: u32 = 25;

#[component]
pub fn AdminAudit() -> impl IntoView {
    // Filter signals
    let actor = RwSignal::new(String::new());
    let action = RwSignal::new(None::<ActionKindDto>);
    let target_ref = RwSignal::new(String::new());
    let result = RwSignal::new(None::<AuditResultDto>);
    let since_epoch_s = RwSignal::new(None::<i64>);
    let until_epoch_s = RwSignal::new(None::<i64>);
    let since_input = RwSignal::new(String::new());
    let until_input = RwSignal::new(String::new());

    let page = RwSignal::new(0u32);

    // Building the filter is a function that reads all signals (so the
    // Resource dependency picks them up).
    let build_filter = move || AuditFilterDto {
        actor_id: {
            let v = actor.get();
            if v.trim().is_empty() { None } else { Some(v.trim().to_string()) }
        },
        action: action.get(),
        target_ref: {
            let v = target_ref.get();
            if v.trim().is_empty() { None } else { Some(v.trim().to_string()) }
        },
        result: result.get(),
        since_epoch_s: since_epoch_s.get(),
        until_epoch_s: until_epoch_s.get(),
    };

    // Resource depends on filter + page; refetches on any change.
    let events_res = Resource::new(
        move || (build_filter(), page.get()),
        |(filter, p)| async move {
            list_audit_events(filter, PAGE_SIZE, p * PAGE_SIZE).await
        },
    );

    let count_res = Resource::new(
        move || build_filter(),
        |filter| async move { count_audit_events(filter).await },
    );

    // Detail modal state
    let detail_event = RwSignal::new(None::<AuditEventRow>);
    let detail_open = RwSignal::new(false);
    let open_detail = move |e: AuditEventRow| {
        detail_event.set(Some(e));
        detail_open.set(true);
    };

    // Time preset helpers — compute epoch seconds for "now - offset".
    let preset_last_hour = move |_| {
        let now = now_epoch_s();
        since_epoch_s.set(Some(now - 3600));
        until_epoch_s.set(None);
        since_input.set(String::new());
        until_input.set(String::new());
        page.set(0);
    };
    let preset_last_24h = move |_| {
        let now = now_epoch_s();
        since_epoch_s.set(Some(now - 86_400));
        until_epoch_s.set(None);
        since_input.set(String::new());
        until_input.set(String::new());
        page.set(0);
    };
    let preset_last_7d = move |_| {
        let now = now_epoch_s();
        since_epoch_s.set(Some(now - 7 * 86_400));
        until_epoch_s.set(None);
        since_input.set(String::new());
        until_input.set(String::new());
        page.set(0);
    };
    let preset_last_30d = move |_| {
        let now = now_epoch_s();
        since_epoch_s.set(Some(now - 30 * 86_400));
        until_epoch_s.set(None);
        since_input.set(String::new());
        until_input.set(String::new());
        page.set(0);
    };
    let preset_all = move |_| {
        since_epoch_s.set(None);
        until_epoch_s.set(None);
        since_input.set(String::new());
        until_input.set(String::new());
        page.set(0);
    };

    // datetime-local input handlers — parse string to epoch seconds.
    let on_since_input = move |ev: leptos::ev::Event| {
        let v = event_target_value(&ev);
        since_input.set(v.clone());
        since_epoch_s.set(parse_datetime_local(&v));
        page.set(0);
    };
    let on_until_input = move |ev: leptos::ev::Event| {
        let v = event_target_value(&ev);
        until_input.set(v.clone());
        until_epoch_s.set(parse_datetime_local(&v));
        page.set(0);
    };

    let total_pages = move || {
        count_res
            .get()
            .and_then(|r| r.ok())
            .map(|n| ((n as u32).max(1).saturating_add(PAGE_SIZE - 1)) / PAGE_SIZE)
            .unwrap_or(1)
            .max(1)
    };

    view! {
        <div>
            <div class="flex items-center justify-between mb-6">
                <h1 class="text-xl font-semibold text-text-primary">"Audit Log"</h1>
                <button
                    type="button"
                    on:click=move |_| { events_res.refetch(); count_res.refetch(); }
                    class="px-3 py-1.5 bg-surface-tertiary border border-border-primary rounded-md text-text-secondary hover:text-text-primary text-sm transition-colors"
                >
                    "Refresh"
                </button>
            </div>

            // Filter form
            <div class="bg-surface-secondary border border-border-primary rounded-md p-4 mb-4 space-y-3">
                <div class="grid grid-cols-2 md:grid-cols-4 gap-3">
                    <div>
                        <label class="block text-xs text-text-muted mb-1">"Actor"</label>
                        <input
                            type="text"
                            class="w-full px-2 py-1.5 bg-surface-tertiary border border-border-primary rounded text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                            placeholder="admin"
                            prop:value=move || actor.get()
                            on:input=move |ev| { actor.set(event_target_value(&ev)); page.set(0); }
                        />
                    </div>
                    <div>
                        <label class="block text-xs text-text-muted mb-1">"Action"</label>
                        <select
                            class="w-full px-2 py-1.5 bg-surface-tertiary border border-border-primary rounded text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                            on:change=move |ev| {
                                let v = event_target_value(&ev);
                                action.set(if v.is_empty() { None } else { Some(parse_action(&v)) });
                                page.set(0);
                            }
                        >
                            <option value="" selected=move || action.get().is_none()>"any"</option>
                            {ActionKindDto::all().iter().map(|a| {
                                let action_kind = *a;
                                let label = action_kind.label();
                                view! {
                                    <option
                                        value=label
                                        selected=move || action.get() == Some(action_kind)
                                    >
                                        {label}
                                    </option>
                                }
                            }).collect_view()}
                        </select>
                    </div>
                    <div>
                        <label class="block text-xs text-text-muted mb-1">"Target ref (substring)"</label>
                        <input
                            type="text"
                            class="w-full px-2 py-1.5 bg-surface-tertiary border border-border-primary rounded text-text-primary text-sm font-mono focus:outline-none focus:border-accent-amber"
                            placeholder="target://..."
                            prop:value=move || target_ref.get()
                            on:input=move |ev| { target_ref.set(event_target_value(&ev)); page.set(0); }
                        />
                    </div>
                    <div>
                        <label class="block text-xs text-text-muted mb-1">"Result"</label>
                        <select
                            class="w-full px-2 py-1.5 bg-surface-tertiary border border-border-primary rounded text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                            on:change=move |ev| {
                                let v = event_target_value(&ev);
                                result.set(match v.as_str() {
                                    "success" => Some(AuditResultDto::Success),
                                    "error" => Some(AuditResultDto::Error),
                                    "denied" => Some(AuditResultDto::Denied),
                                    _ => None,
                                });
                                page.set(0);
                            }
                        >
                            <option value="" selected=move || result.get().is_none()>"any"</option>
                            <option value="success" selected=move || result.get() == Some(AuditResultDto::Success)>"success"</option>
                            <option value="error" selected=move || result.get() == Some(AuditResultDto::Error)>"error"</option>
                            <option value="denied" selected=move || result.get() == Some(AuditResultDto::Denied)>"denied"</option>
                        </select>
                    </div>
                </div>

                // Time range
                <div>
                    <label class="block text-xs text-text-muted mb-1">"Time range"</label>
                    <div class="flex flex-wrap gap-1.5 mb-2">
                        <PresetButton label="Last hour".to_string() on_click=preset_last_hour />
                        <PresetButton label="Last 24h".to_string() on_click=preset_last_24h />
                        <PresetButton label="Last 7d".to_string() on_click=preset_last_7d />
                        <PresetButton label="Last 30d".to_string() on_click=preset_last_30d />
                        <PresetButton label="All time".to_string() on_click=preset_all />
                    </div>
                    <div class="grid grid-cols-2 gap-2">
                        <input
                            type="datetime-local"
                            class="w-full px-2 py-1.5 bg-surface-tertiary border border-border-primary rounded text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                            placeholder="since"
                            prop:value=move || since_input.get()
                            on:input=on_since_input
                        />
                        <input
                            type="datetime-local"
                            class="w-full px-2 py-1.5 bg-surface-tertiary border border-border-primary rounded text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                            placeholder="until"
                            prop:value=move || until_input.get()
                            on:input=on_until_input
                        />
                    </div>
                </div>
            </div>

            // Results table
            <Suspense fallback=|| view! {
                <p class="text-text-muted text-sm">"Loading audit events..."</p>
            }>
                {move || events_res.get().map(|result| match result {
                    Ok(events) if events.is_empty() => view! {
                        <p class="text-text-muted text-sm">"No audit events match the current filter."</p>
                    }.into_any(),
                    Ok(events) => view! {
                        <AuditTable events=events on_row_click=Callback::new(open_detail) />
                    }.into_any(),
                    Err(e) => view! {
                        <p class="text-accent-danger text-sm">{e.to_string()}</p>
                    }.into_any(),
                })}
            </Suspense>

            // Pagination footer
            <div class="flex items-center justify-between mt-4 text-sm text-text-secondary">
                <div>
                    {move || {
                        let total = count_res.get().and_then(|r| r.ok()).unwrap_or(0);
                        let p = page.get();
                        let from = (p * PAGE_SIZE) + 1;
                        let to = ((p + 1) * PAGE_SIZE).min(total as u32);
                        if total == 0 {
                            "0 events".to_string()
                        } else {
                            format!("{} – {} of {}", from, to, total)
                        }
                    }}
                </div>
                <div class="flex items-center gap-2">
                    <button
                        type="button"
                        disabled=move || page.get() == 0
                        on:click=move |_| page.update(|p| if *p > 0 { *p -= 1; })
                        class="px-3 py-1 bg-surface-tertiary border border-border-primary rounded hover:bg-surface-tertiary/80 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                    >
                        "Prev"
                    </button>
                    <span class="text-text-muted">
                        "Page " {move || page.get() + 1} " of " {move || total_pages()}
                    </span>
                    <button
                        type="button"
                        disabled=move || page.get() + 1 >= total_pages()
                        on:click=move |_| page.update(|p| *p += 1)
                        class="px-3 py-1 bg-surface-tertiary border border-border-primary rounded hover:bg-surface-tertiary/80 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                    >
                        "Next"
                    </button>
                </div>
            </div>

            <DetailModal open=detail_open event=detail_event />
        </div>
    }
}

#[component]
fn PresetButton(
    #[prop(into)] label: String,
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static + Send + Sync,
) -> impl IntoView {
    let label_view = label.clone();
    view! {
        <button
            type="button"
            on:click=on_click
            class="px-2.5 py-1 bg-surface-tertiary border border-border-primary rounded text-text-secondary hover:text-text-primary hover:bg-surface-tertiary/80 text-xs transition-colors"
        >
            {label_view}
        </button>
    }
}

#[component]
fn AuditTable(
    events: Vec<AuditEventRow>,
    on_row_click: Callback<AuditEventRow>,
) -> impl IntoView {
    view! {
        <div class="overflow-x-auto border border-border-primary rounded-md">
            <table class="w-full text-sm">
                <thead class="bg-surface-secondary text-text-muted text-xs uppercase tracking-wider">
                    <tr>
                        <th class="px-3 py-2 text-left">"Time"</th>
                        <th class="px-3 py-2 text-left">"Actor"</th>
                        <th class="px-3 py-2 text-left">"Action"</th>
                        <th class="px-3 py-2 text-left">"Result"</th>
                        <th class="px-3 py-2 text-left">"Target"</th>
                        <th class="px-3 py-2 text-right">"Latency"</th>
                    </tr>
                </thead>
                <tbody>
                    {events.into_iter().map(|e| {
                        let e_for_click = e.clone();
                        let result_color = match e.result {
                            AuditResultDto::Success => "bg-accent-green",
                            AuditResultDto::Error => "bg-accent-danger",
                            AuditResultDto::Denied => "bg-accent-amber",
                        };
                        view! {
                            <tr
                                class="border-t border-border-primary hover:bg-surface-tertiary cursor-pointer transition-colors"
                                on:click=move |_| on_row_click.run(e_for_click.clone())
                            >
                                <td class="px-3 py-2 font-mono text-[12px] text-text-muted whitespace-nowrap">
                                    {short_ts(&e.ts_rfc3339)}
                                </td>
                                <td class="px-3 py-2 text-text-primary text-[13px]">{e.actor_id.clone()}</td>
                                <td class="px-3 py-2 font-mono text-[12px] text-text-secondary">{e.action.label()}</td>
                                <td class="px-3 py-2">
                                    <span class="inline-flex items-center gap-1.5 text-[12px]">
                                        <span class=format!("w-2 h-2 rounded-full {}", result_color)></span>
                                        {e.result.label()}
                                    </span>
                                </td>
                                <td class="px-3 py-2 font-mono text-[12px] text-text-secondary">
                                    {e.target_ref.clone().unwrap_or_else(|| "—".to_string())}
                                </td>
                                <td class="px-3 py-2 text-right text-text-muted text-[12px] font-mono">
                                    {match e.latency_ms {
                                        Some(ms) => format!("{} ms", ms),
                                        None => "—".to_string(),
                                    }}
                                </td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
}

#[component]
fn DetailModal(
    open: RwSignal<bool>,
    event: RwSignal<Option<AuditEventRow>>,
) -> impl IntoView {
    view! {
        <Modal title="Audit Event".to_string() open=open max_width="max-w-2xl">
            {move || event.get().map(|e| view! {
                <div class="space-y-3">
                    <DetailRow label="Time".to_string() value=e.ts_rfc3339.clone() mono=true />
                    <DetailRow label="Actor".to_string() value=e.actor_id.clone() mono=false />
                    <DetailRow label="Action".to_string() value=e.action.label().to_string() mono=true />
                    <DetailRow label="Result".to_string() value=e.result.label().to_string() mono=false />
                    {e.target_ref.clone().map(|t| view! { <DetailRow label="Target ref".to_string() value=t mono=true /> })}
                    {e.credential_ref.clone().map(|c| view! { <DetailRow label="Credential ref".to_string() value=c mono=true /> })}
                    {e.op_summary.clone().map(|s| view! { <DetailRow label="Op summary".to_string() value=s mono=false /> })}
                    {e.latency_ms.map(|ms| view! { <DetailRow label="Latency".to_string() value=format!("{} ms", ms) mono=true /> })}
                    {if !e.metadata.is_empty() {
                        let rows = e.metadata.clone();
                        view! {
                            <div>
                                <label class="block text-xs text-text-muted mb-1">"Metadata"</label>
                                <div class="bg-surface-tertiary border border-border-primary rounded-md p-3 space-y-1 text-xs font-mono">
                                    {rows.into_iter().map(|(k, v)| view! {
                                        <div class="flex gap-2">
                                            <span class="text-text-muted">{k}":"</span>
                                            <span class="text-text-primary break-all">{v}</span>
                                        </div>
                                    }).collect_view()}
                                </div>
                            </div>
                        }.into_any()
                    } else {
                        view! {}.into_any()
                    }}
                    <DetailRow label="ID".to_string() value=e.id.to_string() mono=true />
                    <div class="flex justify-end pt-2">
                        <button
                            type="button"
                            on:click=move |_| open.set(false)
                            class="px-4 py-2 bg-surface-tertiary border border-border-primary rounded-md hover:bg-surface-tertiary/80 transition-colors text-sm text-text-secondary"
                        >
                            "Close"
                        </button>
                    </div>
                </div>
            })}
        </Modal>
    }
}

#[component]
fn DetailRow(
    #[prop(into)] label: String,
    #[prop(into)] value: String,
    #[prop(default = false)] mono: bool,
) -> impl IntoView {
    let label_view = label.clone();
    let value_view = value.clone();
    let class = if mono {
        "w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-xs font-mono break-all"
    } else {
        "w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm"
    };
    view! {
        <div>
            <label class="block text-xs text-text-muted mb-1">{label_view}</label>
            <div class=class>{value_view}</div>
        </div>
    }
}

// -------- Helpers -----------------------------------------------------------

fn short_ts(rfc3339: &str) -> String {
    if let Some((date, rest)) = rfc3339.split_once('T') {
        let hms = rest.get(0..8).unwrap_or(rest);
        format!("{} {}", date, hms)
    } else {
        rfc3339.to_string()
    }
}

fn parse_action(label: &str) -> ActionKindDto {
    match label {
        "broker.execute" => ActionKindDto::BrokerExecute,
        "vault.resolve" => ActionKindDto::VaultResolve,
        "vault.reveal" => ActionKindDto::VaultReveal,
        "vault.create" => ActionKindDto::VaultCreate,
        "vault.update" => ActionKindDto::VaultUpdate,
        "vault.rename" => ActionKindDto::VaultRename,
        "vault.delete" => ActionKindDto::VaultDelete,
        "inventory.upsert" => ActionKindDto::InventoryUpsert,
        "inventory.remove" => ActionKindDto::InventoryRemove,
        "inventory.resolve" => ActionKindDto::InventoryResolve,
        "transport.dispatch" => ActionKindDto::TransportDispatch,
        "guard.approve" => ActionKindDto::GuardApprove,
        "guard.deny" => ActionKindDto::GuardDeny,
        _ => ActionKindDto::Other,
    }
}

/// Current time as epoch seconds. SSR side returns 0 (the page never renders
/// reactively server-side; the user always loads the page client-side).
fn now_epoch_s() -> i64 {
    #[cfg(feature = "hydrate")]
    {
        (js_sys::Date::now() / 1000.0) as i64
    }
    #[cfg(not(feature = "hydrate"))]
    {
        0
    }
}

/// Parse a `<input type="datetime-local">` value (e.g. "2026-05-22T22:30")
/// into epoch seconds. The browser interprets the local time correctly via
/// `Date.parse`. Empty string returns None.
fn parse_datetime_local(s: &str) -> Option<i64> {
    if s.trim().is_empty() {
        return None;
    }
    #[cfg(feature = "hydrate")]
    {
        let ms = js_sys::Date::parse(s);
        if ms.is_nan() {
            None
        } else {
            Some((ms / 1000.0) as i64)
        }
    }
    #[cfg(not(feature = "hydrate"))]
    {
        let _ = s;
        None
    }
}

