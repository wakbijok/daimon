//! UI-2 — schema-driven settings forms (console v2, Wak's rule: "bukan free form
//! fill. kena creative between toggle, drop down, text field").
//!
//! Each editable domain declares a small typed schema (control kind + label +
//! help + options). `FormTab` renders those as real controls, seeds them from
//! `list_settings(prefix)`, and writes through `set_setting` — so the P6-3
//! server-side vault interception (secrets → `vault://` refs) is untouched. The
//! raw key/value editor survives only as a collapsed "Advanced" escape hatch.
//! The consumed-key registry (`config_keys`) remains the source of truth; these
//! schemas are its human-facing shape.

use leptos::prelude::*;
use serde_json::Value as Json;
use std::collections::HashMap;

use crate::admin_settings::{list_settings, set_setting, SettingRow};

/// The control a field renders as.
#[derive(Clone)]
pub enum Control {
    Toggle,
    Text { placeholder: &'static str },
    Number { min: i64, max: i64, unit: &'static str },
    Select(&'static [&'static str]),
    Segmented(&'static [&'static str]),
    Secret,
}

#[derive(Clone)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub control: Control,
}

impl Field {
    fn is_secret(&self) -> bool {
        matches!(self.control, Control::Secret)
    }
}

/// Domain schemas. LLM/AI is a dedicated page (UI-3); channels a custom tab; so
/// the form framework covers the plain typed domains here.
pub fn schema_for(prefix: &str) -> Vec<Field> {
    match prefix {
        "identity." => vec![Field {
            key: "identity.org_name",
            label: "Organisation name",
            help: "Shown in the console header.",
            control: Control::Text { placeholder: "daimon" },
        }],
        "guard." => vec![
            Field {
                key: "guard.approval_timeout_secs",
                label: "Approval timeout",
                help: "How long a gated write waits for a decision before it is denied. Applies to the next gated request.",
                control: Control::Number { min: 10, max: 86400, unit: "seconds" },
            },
            Field {
                key: "guard.blast_radius_depth",
                label: "Blast-radius depth",
                help: "Graph traversal depth shown with an approval so the operator sees what a write touches.",
                control: Control::Number { min: 1, max: 12, unit: "hops" },
            },
        ],
        "observer." => vec![Field {
            key: "observer.prom_poll_interval_secs",
            label: "Prometheus poll interval",
            help: "How often the observer queries Prometheus. Applies on the next tick.",
            control: Control::Number { min: 5, max: 3600, unit: "seconds" },
        }],
        "chat." => vec![Field {
            key: "chat.history_retention_days",
            label: "Chat history retention",
            help: "Days to keep chat transcripts. 0 = keep forever. Independent of the login-session lifetime.",
            control: Control::Number { min: 0, max: 3650, unit: "days" },
        }],
        _ => Vec::new(),
    }
}

/// True if a schema-driven form exists for this prefix.
pub fn has_form(prefix: &str) -> bool {
    !schema_for(prefix).is_empty()
}

fn row_value(rows: &[SettingRow], key: &str) -> String {
    rows.iter()
        .find(|r| r.key == key)
        .map(|r| match &r.value {
            Json::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

#[component]
pub fn FormTab(prefix: &'static str, title: &'static str) -> impl IntoView {
    let fields = StoredValue::new(schema_for(prefix));
    let settings = Resource::new(move || prefix, |p| list_settings(p.to_string()));
    let (status, set_status) = signal::<Option<String>>(None);

    // A local editable value per field key, seeded once the current values load.
    let values = StoredValue::new(HashMap::<&'static str, RwSignal<String>>::new());
    values.update_value(|m| {
        for f in schema_for(prefix) {
            m.insert(f.key, RwSignal::new(String::new()));
        }
    });

    // Seed from the server once loaded.
    Effect::new(move |_| {
        if let Some(Ok(rows)) = settings.get() {
            values.with_value(|m| {
                for (key, sig) in m.iter() {
                    // Do not overwrite a secret's stored ref into the input — leave
                    // secret inputs blank (a blank secret on save = unchanged).
                    let is_secret = fields.with_value(|fs| {
                        fs.iter().find(|f| f.key == *key).map(Field::is_secret).unwrap_or(false)
                    });
                    if !is_secret {
                        sig.set(row_value(&rows, key));
                    }
                }
            });
        }
    });

    let save = Action::new(move |_: &()| async move {
        let mut items: Vec<(String, Json, bool)> = Vec::new();
        fields.with_value(|fs| {
            values.with_value(|m| {
                for f in fs {
                    if let Some(sig) = m.get(f.key) {
                        let raw = sig.get_untracked();
                        // A blank secret means "unchanged" — skip it.
                        if f.is_secret() && raw.trim().is_empty() {
                            continue;
                        }
                        let val = to_json(&f.control, &raw);
                        items.push((f.key.to_string(), val, f.is_secret()));
                    }
                }
            });
        });
        let mut ok = 0;
        let mut err: Option<String> = None;
        for (k, v, sec) in items {
            match set_setting(k, v, sec).await {
                Ok(_) => ok += 1,
                Err(e) => err = Some(format!("{e}")),
            }
        }
        match err {
            Some(e) => format!("error: {e}"),
            None => format!("saved {ok} setting(s) ✓"),
        }
    });
    Effect::new(move |_| {
        if let Some(msg) = save.value().get() {
            set_status.set(Some(msg));
            settings.refetch();
        }
    });

    view! {
        <div class="space-y-5 max-w-3xl">
            <div class="flex items-center gap-3">
                <h2 class="text-lg font-semibold text-text-primary">{title}</h2>
                {move || status.get().map(|s| view! {
                    <span class="text-xs font-mono text-text-secondary">{s}</span>
                })}
            </div>

            <Suspense fallback=|| view! { <div class="text-text-muted text-sm">"loading…"</div> }>
                {move || settings.get().map(|_| {
                    fields.get_value().into_iter().map(|f| {
                        let sig = values.with_value(|m| *m.get(f.key).unwrap());
                        view! { <FieldRow field=f value=sig /> }
                    }).collect_view()
                })}
            </Suspense>

            <div class="flex justify-end pt-1">
                <button
                    on:click=move |_| { save.dispatch(()); }
                    class="px-4 py-1.5 bg-accent-amber text-surface-primary font-medium rounded-md text-sm"
                >
                    "Save changes"
                </button>
            </div>

            <AdvancedRaw prefix=prefix />
        </div>
    }
}

fn to_json(control: &Control, raw: &str) -> Json {
    match control {
        Control::Toggle => Json::Bool(raw == "true"),
        Control::Number { .. } => raw
            .parse::<i64>()
            .map(|n| Json::Number(n.into()))
            .unwrap_or_else(|_| Json::String(raw.to_string())),
        _ => Json::String(raw.to_string()),
    }
}

#[component]
fn FieldRow(field: Field, value: RwSignal<String>) -> impl IntoView {
    let label = field.label;
    let help = field.help;
    let control = field.control.clone();
    view! {
        <div class="grid gap-4 py-3 border-b border-border-primary/40" style="grid-template-columns:230px 1fr">
            <div class="pt-1.5">
                <div class="text-[13px] font-semibold text-text-primary">{label}</div>
                <div class="text-[11.5px] text-text-muted mt-0.5 leading-snug">{help}</div>
            </div>
            <div class="max-w-md">{render_control(control, value, field.is_secret(), field.key)}</div>
        </div>
    }
}

fn render_control(control: Control, value: RwSignal<String>, is_secret: bool, key: &'static str) -> AnyView {
    match control {
        Control::Toggle => view! {
            <label class="inline-flex items-center gap-3 cursor-pointer">
                <input type="checkbox"
                    prop:checked=move || value.get() == "true"
                    on:change=move |ev| value.set(if event_target_checked(&ev) { "true".into() } else { "false".into() })
                    class="w-4 h-4 accent-accent-amber" />
                <span class="text-sm text-text-secondary">{move || if value.get() == "true" { "Enabled" } else { "Disabled" }}</span>
            </label>
        }.into_any(),
        Control::Number { min, max, unit } => view! {
            <div class="flex items-center gap-2">
                <input type="number" min=min.to_string() max=max.to_string()
                    class="w-32 px-3 py-1.5 bg-bg border border-border-primary rounded-md text-sm text-text-primary font-mono focus:outline-none focus:border-accent-amber"
                    prop:value=move || value.get()
                    on:input=move |ev| value.set(event_target_value(&ev)) />
                <span class="text-xs text-text-muted">{unit}</span>
            </div>
        }.into_any(),
        Control::Select(opts) => view! {
            <select class="w-full px-3 py-1.5 bg-bg border border-border-primary rounded-md text-sm text-text-primary focus:outline-none focus:border-accent-amber"
                on:change=move |ev| value.set(event_target_value(&ev))
                prop:value=move || value.get()>
                {opts.iter().map(|o| { let o=*o; view! { <option value=o>{o}</option> } }).collect_view()}
            </select>
        }.into_any(),
        Control::Segmented(opts) => view! {
            <div class="inline-flex border border-border-primary rounded-md overflow-hidden">
                {opts.iter().map(|o| {
                    let o = *o;
                    view! {
                        <button
                            on:click=move |_| value.set(o.to_string())
                            class=move || format!(
                                "px-3 py-1.5 text-xs font-mono transition-colors {}",
                                if value.get() == o { "bg-accent-amber/15 text-accent-amber" } else { "text-text-muted hover:text-text-primary" }
                            )
                        >{o}</button>
                    }
                }).collect_view()}
            </div>
        }.into_any(),
        Control::Text { placeholder } => view! {
            <input type="text" placeholder=placeholder
                class="w-full px-3 py-1.5 bg-bg border border-border-primary rounded-md text-sm text-text-primary focus:outline-none focus:border-accent-amber"
                prop:value=move || value.get()
                on:input=move |ev| value.set(event_target_value(&ev)) />
        }.into_any(),
        Control::Secret => view! {
            <div class="space-y-1.5">
                <input type="password" placeholder="•••••••• (leave blank to keep current)"
                    class="w-full px-3 py-1.5 bg-bg border border-border-primary rounded-md text-sm text-text-primary font-mono focus:outline-none focus:border-accent-amber"
                    prop:value=move || value.get()
                    on:input=move |ev| value.set(event_target_value(&ev)) />
                <span class="inline-flex items-center gap-1.5 text-[10.5px] font-mono text-emerald-400">
                    {format!("🔒 stored as vault://settings.{key}")}
                </span>
            </div>
        }.into_any(),
        #[allow(unreachable_patterns)]
        _ => { let _ = is_secret; view! { <span/> }.into_any() }
    }
}

/// The raw key/value editor, demoted to a collapsed escape hatch.
#[component]
fn AdvancedRaw(prefix: &'static str) -> impl IntoView {
    let settings = Resource::new(move || prefix, |p| list_settings(p.to_string()));
    view! {
        <details class="mt-4 rounded-lg border border-border-primary bg-surface-secondary/40">
            <summary class="px-3 py-2 text-xs text-text-secondary cursor-pointer select-none">
                "Advanced — raw keys under " <code class="text-accent-amber">{prefix}</code>
            </summary>
            <div class="px-3 pb-3">
                <Suspense fallback=|| view! { <div class="text-text-muted text-xs py-2">"loading…"</div> }>
                    {move || settings.get().map(|res| match res {
                        Ok(rows) if rows.is_empty() => view! { <div class="text-text-muted text-xs py-2">"no raw keys set"</div> }.into_any(),
                        Ok(rows) => view! {
                            <table class="w-full text-xs">
                                <tbody>
                                    {rows.into_iter().map(|r| {
                                        let val = if r.is_secret { "•••••• (vault ref)".to_string() } else {
                                            match &r.value { Json::String(s) => s.clone(), o => o.to_string() }
                                        };
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
