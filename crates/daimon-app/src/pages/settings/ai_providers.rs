//! UI-3 — AI Providers, the Hermes-style provider catalog (console v2).
//!
//! Wak's rule: "untuk AI provider, boleh tengok macam mana hermes buat, banyak
//! betul integration dia provide." We take the Hermes *UX* — a visual catalog of
//! providers, each with an **auth-type-aware** config form — and apply it to the
//! providers daimon actually consumes. Honesty first (the same discipline as
//! P6-6): the runtime resolves exactly four provider families in
//! `chat::select_llm` (anthropic / openai API-key, chatgpt OAuth-session,
//! local/ollama base-URL), so the catalog shows exactly those. Adding a provider
//! is adding a `ProviderSpec` here + a branch there — never a dead card.
//!
//! Every write goes through `set_setting`, so the P6-3 server-side vault
//! interception (API keys → `vault://` refs) stays intact: a blank secret field
//! means "unchanged", a filled one is intercepted and only its ref persisted.

use leptos::prelude::*;
use serde_json::Value as Json;

use crate::admin_settings::{SettingRow, list_settings, set_setting};

/// How a provider authenticates — drives which controls its config form renders.
#[derive(Clone, Copy)]
enum Auth {
    /// A bearer API key held as a `vault://` ref (secret).
    ApiKey {
        key_setting: &'static str,
        env_hint: &'static str,
    },
    /// An externally-managed OAuth session (Codex) read from the environment —
    /// no key is stored by daimon.
    OAuthSession { note: &'static str },
    /// A self-hosted endpoint reached by base URL — no credential.
    LocalUrl {
        url_setting: &'static str,
        default_url: &'static str,
    },
}

#[derive(Clone, Copy)]
struct ProviderSpec {
    /// The value written to `llm.provider` when this provider is activated. Must
    /// match a branch in `chat::select_llm`.
    id: &'static str,
    name: &'static str,
    tagline: &'static str,
    /// Short mark shown in the glyph tile.
    glyph: &'static str,
    /// Tailwind text/border accent class for the glyph tile.
    accent: &'static str,
    default_model: &'static str,
    auth: Auth,
}

/// The catalog. Order = anthropic (default), openai, chatgpt, local.
const PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec {
        id: "anthropic",
        name: "Anthropic",
        tagline: "Claude — the compiled default. Bearer API key.",
        glyph: "An",
        accent: "text-accent-amber border-accent-amber/40",
        default_model: "claude-sonnet-4-6",
        auth: Auth::ApiKey {
            key_setting: "llm.anthropic_key",
            env_hint: "ANTHROPIC_API_KEY",
        },
    },
    ProviderSpec {
        id: "openai",
        name: "OpenAI",
        tagline: "GPT models over the OpenAI API. Bearer API key.",
        glyph: "AI",
        accent: "text-accent-green border-accent-green/40",
        default_model: "gpt-4o",
        auth: Auth::ApiKey {
            key_setting: "llm.openai_key",
            env_hint: "OPENAI_API_KEY",
        },
    },
    ProviderSpec {
        id: "chatgpt",
        name: "ChatGPT (OAuth)",
        tagline: "Codex OAuth session — no API key stored. Supports per-turn effort.",
        glyph: "GPT",
        accent: "text-accent-purple border-accent-purple/40",
        default_model: "",
        auth: Auth::OAuthSession {
            note: "Reads the Codex OAuth session from the environment at boot. daimon stores no \
                   credential for this provider — sign in with the Codex CLI on the host, then \
                   activate here.",
        },
    },
    ProviderSpec {
        id: "local",
        name: "Local / Ollama",
        tagline: "Self-hosted models over an OpenAI-compatible endpoint. No credential.",
        glyph: "◇",
        accent: "text-text-secondary border-border-primary",
        default_model: "llama3.2",
        auth: Auth::LocalUrl {
            url_setting: "llm.ollama_url",
            default_url: "http://localhost:11434",
        },
    },
];

/// Normalise a raw `llm.provider` string to a catalog id, mirroring the matching
/// in `chat::select_llm` (unknown → anthropic default; `ollama` → `local`).
fn active_id(raw: &str) -> &'static str {
    match raw.to_ascii_lowercase().as_str() {
        "openai" => "openai",
        "chatgpt" => "chatgpt",
        "local" | "ollama" => "local",
        _ => "anthropic",
    }
}

fn row<'a>(rows: &'a [SettingRow], key: &str) -> Option<&'a SettingRow> {
    rows.iter().find(|r| r.key == key)
}

fn row_string(rows: &[SettingRow], key: &str) -> String {
    row(rows, key)
        .map(|r| match &r.value {
            Json::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

/// Is this provider "configured" — enough that activating it would resolve a
/// client? API-key providers need the key present (as a vault ref); local needs
/// nothing (has a default URL); OAuth is host-managed so we can't see it.
fn is_configured(spec: &ProviderSpec, rows: &[SettingRow]) -> Option<bool> {
    match spec.auth {
        Auth::ApiKey { key_setting, .. } => Some(row(rows, key_setting).is_some()),
        Auth::LocalUrl { .. } => Some(true),
        Auth::OAuthSession { .. } => None, // unknowable from here
    }
}

#[component]
pub fn AiProviders() -> impl IntoView {
    let settings = LocalResource::new(move || list_settings("llm.".to_string()));
    let (status, set_status) = signal::<Option<String>>(None);
    // Which provider's config panel is open. Seeded to the active provider once
    // settings load; the operator can click any card to switch focus.
    let selected: RwSignal<Option<&'static str>> = RwSignal::new(None);

    Effect::new(move |_| {
        if let Some(Ok(rows)) = settings.get() {
            if selected.get_untracked().is_none() {
                let active = active_id(&row_string(&rows, "llm.provider"));
                selected.set(Some(active));
            }
        }
    });

    view! {
        <div class="max-w-4xl space-y-6">
            <div class="flex items-start justify-between gap-4">
                <div>
                    <h2 class="text-lg font-semibold text-text-primary">"AI Providers"</h2>
                    <p class="text-[12px] text-text-muted mt-0.5 max-w-xl leading-snug">
                        "The model behind chat and every agent turn. Pick a provider, give it \
                         credentials, and make it active — the change is live on the next turn. \
                         Keys are stored in the vault; only a reference is kept here."
                    </p>
                </div>
                {move || status.get().map(|s| view! {
                    <span class="text-xs font-mono text-text-secondary whitespace-nowrap pt-1">{s}</span>
                })}
            </div>

            <Suspense fallback=|| view! {
                <div class="text-text-muted text-sm py-8 text-center">"loading providers…"</div>
            }>
                {move || settings.get().map(|res| match res {
                    Err(e) => view! {
                        <div class="text-accent-danger text-sm">{format!("error: {e}")}</div>
                    }.into_any(),
                    Ok(rows) => {
                        let active = active_id(&row_string(&rows, "llm.provider"));
                        let rows_grid = rows.clone();
                        let rows_panel = rows.clone();
                        view! {
                            // ---- catalog grid ----------------------------------
                            <div class="grid grid-cols-1 sm:grid-cols-2 gap-3">
                                {PROVIDERS.iter().map(|spec| {
                                    let spec = *spec;
                                    let configured = is_configured(&spec, &rows_grid);
                                    let is_active = active == spec.id;
                                    view! {
                                        <ProviderCard
                                            spec=spec
                                            is_active=is_active
                                            configured=configured
                                            selected=selected
                                        />
                                    }
                                }).collect_view()}
                            </div>

                            // ---- auth-aware config panel for the focused card --
                            {move || {
                                let sel = selected.get();
                                let rows = rows_panel.clone();
                                sel.and_then(|id| PROVIDERS.iter().find(|p| p.id == id).copied())
                                    .map(|spec| view! {
                                        <ProviderConfig
                                            spec=spec
                                            active_id=active
                                            rows=rows.clone()
                                            settings=settings
                                            set_status=set_status
                                        />
                                    })
                            }}

                            // ---- operator model access (global permit list) ----
                            <ModelAccess rows=rows.clone() settings=settings set_status=set_status />
                        }.into_any()
                    }
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn ProviderCard(
    spec: ProviderSpec,
    is_active: bool,
    configured: Option<bool>,
    selected: RwSignal<Option<&'static str>>,
) -> impl IntoView {
    let id = spec.id;
    let is_selected = Memo::new(move |_| selected.get() == Some(id));
    view! {
        <button
            on:click=move |_| selected.set(Some(id))
            class=move || format!(
                "text-left rounded-lg border p-4 transition-colors w-full {}",
                if is_selected.get() {
                    "border-accent-amber bg-accent-amber/5"
                } else {
                    "border-border-primary bg-surface-secondary hover:border-text-muted"
                }
            )
        >
            <div class="flex items-start gap-3">
                <div class=format!(
                    "shrink-0 w-11 h-11 rounded-md border flex items-center justify-center font-semibold text-sm bg-surface-tertiary {}",
                    spec.accent
                )>
                    {spec.glyph}
                </div>
                <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2">
                        <span class="text-sm font-semibold text-text-primary">{spec.name}</span>
                        {is_active.then(|| view! {
                            <span class="text-[9.5px] font-semibold uppercase tracking-wider px-1.5 py-0.5 rounded bg-accent-amber text-surface-primary">
                                "Active"
                            </span>
                        })}
                    </div>
                    <p class="text-[11.5px] text-text-muted mt-1 leading-snug">{spec.tagline}</p>
                    <div class="mt-2">
                        {match configured {
                            Some(true) => view! {
                                <span class="inline-flex items-center gap-1 text-[10.5px] text-accent-green">
                                    <span class="w-1.5 h-1.5 rounded-full bg-accent-green"></span>
                                    "Configured"
                                </span>
                            }.into_any(),
                            // Precise wording: we only inspect app_config here.
                            // The runtime also falls back to the provider env var
                            // (ANTHROPIC_API_KEY/OPENAI_API_KEY), which the wasm
                            // client can't see — so "no stored key" is honest where
                            // "not configured" would be a false negative.
                            Some(false) => view! {
                                <span class="inline-flex items-center gap-1 text-[10.5px] text-text-muted">
                                    <span class="w-1.5 h-1.5 rounded-full bg-text-muted"></span>
                                    "No stored key"
                                </span>
                            }.into_any(),
                            None => view! {
                                <span class="inline-flex items-center gap-1 text-[10.5px] text-accent-purple">
                                    <span class="w-1.5 h-1.5 rounded-full bg-accent-purple"></span>
                                    "Host-managed session"
                                </span>
                            }.into_any(),
                        }}
                    </div>
                </div>
            </div>
        </button>
    }
}

#[component]
fn ProviderConfig(
    spec: ProviderSpec,
    active_id: &'static str,
    rows: Vec<SettingRow>,
    settings: LocalResource<Result<Vec<SettingRow>, ServerFnError>>,
    set_status: WriteSignal<Option<String>>,
) -> impl IntoView {
    let is_active = active_id == spec.id;

    // Seed per-provider inputs from the loaded rows. Secret inputs stay blank
    // (blank = unchanged); a stored key shows a "key set" chip instead.
    let key_set =
        matches!(spec.auth, Auth::ApiKey { key_setting, .. } if row(&rows, key_setting).is_some());
    // `llm.default_model.chat` is a SINGLE global key, so it only describes the
    // *active* provider's model. Seeding a non-active provider's field from it
    // would carry e.g. "gpt-4o" into the Anthropic form; saving would then write
    // an invalid model for the newly-activated provider and break the next turn.
    // So: seed from the stored key only when configuring the already-active
    // provider; otherwise seed this provider's own compiled default.
    let seed_model = if is_active {
        let m = row_string(&rows, "llm.default_model.chat");
        if m.is_empty() {
            spec.default_model.to_string()
        } else {
            m
        }
    } else {
        spec.default_model.to_string()
    };
    let seed_url = match spec.auth {
        Auth::LocalUrl {
            url_setting,
            default_url,
        } => {
            let u = row_string(&rows, url_setting);
            if u.is_empty() {
                default_url.to_string()
            } else {
                u
            }
        }
        _ => String::new(),
    };

    let api_key = RwSignal::new(String::new());
    let model = RwSignal::new(seed_model);
    let base_url = RwSignal::new(seed_url);

    // Save writes the provider's credential/endpoint + model FIRST, and only
    // flips `llm.provider` if every one of those succeeds — so a failed key
    // write can never leave the active provider pointing at a broken config
    // (activation is the last, gated step, not one write among equals).
    let save = Action::new(move |_: &()| async move {
        let mut config_writes: Vec<(String, Json, bool)> = Vec::new();

        // Provider-specific credential / endpoint.
        match spec.auth {
            Auth::ApiKey { key_setting, .. } => {
                let k = api_key.get_untracked();
                if !k.trim().is_empty() {
                    config_writes.push((key_setting.to_string(), Json::String(k), true));
                }
            }
            Auth::LocalUrl { url_setting, .. } => {
                let u = base_url.get_untracked();
                if !u.trim().is_empty() {
                    config_writes.push((url_setting.to_string(), Json::String(u), false));
                }
            }
            Auth::OAuthSession { .. } => {}
        }

        // Default chat model (skip for OAuth, which manages its own default).
        let m = model.get_untracked();
        if !matches!(spec.auth, Auth::OAuthSession { .. }) && !m.trim().is_empty() {
            config_writes.push(("llm.default_model.chat".to_string(), Json::String(m), false));
        }

        // 1. Credentials/model — abort BEFORE activating if any fails.
        for (k, v, sec) in config_writes {
            if let Err(e) = set_setting(k, v, sec).await {
                return format!("error: {e} — provider NOT switched");
            }
        }

        // 2. Activation — the config is known-good, so flip the active provider.
        match set_setting(
            "llm.provider".to_string(),
            Json::String(spec.id.to_string()),
            false,
        )
        .await
        {
            Ok(_) => format!("{} is now the active provider ✓", spec.name),
            Err(e) => format!("config saved, but activation failed: {e}"),
        }
    });

    Effect::new(move |_| {
        if let Some(msg) = save.value().get() {
            set_status.set(Some(msg));
            settings.refetch();
        }
    });

    view! {
        <div class="rounded-lg border border-border-primary bg-surface-secondary/60 p-5 space-y-4">
            <div class="flex items-center gap-2">
                <h3 class="text-sm font-semibold text-text-primary">
                    "Configure " {spec.name}
                </h3>
                {is_active.then(|| view! {
                    <span class="text-[9.5px] font-semibold uppercase tracking-wider px-1.5 py-0.5 rounded bg-accent-amber/20 text-accent-amber">
                        "Currently active"
                    </span>
                })}
            </div>

            {match spec.auth {
                Auth::ApiKey { key_setting, env_hint } => view! {
                    <div class="space-y-4">
                        <FieldShell label="API key" help=move || format!(
                            "Stored in the vault as a reference. Leave blank to keep the current key. Falls back to {env_hint} if unset."
                        )>
                            <input type="password"
                                placeholder=if key_set { "•••••••• (key set — blank keeps it)" } else { "sk-…" }
                                class="w-full px-3 py-1.5 bg-surface-tertiary border border-border-primary rounded-md text-sm text-text-primary font-mono focus:outline-none focus:border-accent-amber"
                                prop:value=move || api_key.get()
                                on:input=move |ev| api_key.set(event_target_value(&ev)) />
                            <span class="inline-flex items-center gap-1.5 text-[10.5px] font-mono text-accent-green mt-1">
                                {format!("🔒 vault://settings.{key_setting}")}
                                {key_set.then(|| view! { <span class="text-text-muted">" · key on file"</span> })}
                            </span>
                        </FieldShell>
                        <ModelField model=model default_model=spec.default_model />
                    </div>
                }.into_any(),
                Auth::LocalUrl { .. } => view! {
                    <div class="space-y-4">
                        <FieldShell label="Base URL" help=|| "OpenAI-compatible endpoint (Ollama, vLLM, LM Studio, …).".to_string()>
                            <input type="text" placeholder="http://localhost:11434"
                                class="w-full px-3 py-1.5 bg-surface-tertiary border border-border-primary rounded-md text-sm text-text-primary font-mono focus:outline-none focus:border-accent-amber"
                                prop:value=move || base_url.get()
                                on:input=move |ev| base_url.set(event_target_value(&ev)) />
                        </FieldShell>
                        <ModelField model=model default_model=spec.default_model />
                    </div>
                }.into_any(),
                Auth::OAuthSession { note } => view! {
                    <div class="rounded-md border border-accent-purple/30 bg-accent-purple/5 p-3 text-[12px] text-text-secondary leading-snug">
                        {note}
                    </div>
                }.into_any(),
            }}

            <div class="flex items-center gap-3 pt-1">
                <button
                    on:click=move |_| { save.dispatch(()); }
                    class="px-4 py-1.5 bg-accent-amber text-surface-primary font-medium rounded-md text-sm"
                >
                    {if is_active { "Save changes" } else { "Save & make active" }}
                </button>
                {(!is_active).then(|| view! {
                    <span class="text-[11px] text-text-muted">
                        "Switching the active provider takes effect on the next turn."
                    </span>
                })}
            </div>
        </div>
    }
}

/// A label + help + control row, matching the FormTab field layout.
#[component]
fn FieldShell(
    label: &'static str,
    #[prop(into)] help: Signal<String>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="grid gap-4" style="grid-template-columns:180px 1fr">
            <div class="pt-1.5">
                <div class="text-[13px] font-semibold text-text-primary">{label}</div>
                <div class="text-[11px] text-text-muted mt-0.5 leading-snug">{move || help.get()}</div>
            </div>
            <div class="max-w-md">{children()}</div>
        </div>
    }
}

#[component]
fn ModelField(model: RwSignal<String>, default_model: &'static str) -> impl IntoView {
    view! {
        <FieldShell label="Default model" help=|| "The model used for the chat/worker role. Live on the next turn.".to_string()>
            <input type="text" placeholder=default_model
                class="w-full px-3 py-1.5 bg-surface-tertiary border border-border-primary rounded-md text-sm text-text-primary font-mono focus:outline-none focus:border-accent-amber"
                prop:value=move || model.get()
                on:input=move |ev| model.set(event_target_value(&ev)) />
        </FieldShell>
    }
}

/// The operator model-access permit list (`llm.available_models`) — which models
/// a chat operator may pick, enforced server-side. Empty = default model only.
#[component]
fn ModelAccess(
    rows: Vec<SettingRow>,
    settings: LocalResource<Result<Vec<SettingRow>, ServerFnError>>,
    set_status: WriteSignal<Option<String>>,
) -> impl IntoView {
    let csv = RwSignal::new(row_string(&rows, "llm.available_models"));
    let save = Action::new(move |_: &()| async move {
        let v = csv.get_untracked();
        match set_setting("llm.available_models".to_string(), Json::String(v), false).await {
            Ok(_) => "model access saved ✓".to_string(),
            Err(e) => format!("error: {e}"),
        }
    });
    Effect::new(move |_| {
        if let Some(msg) = save.value().get() {
            set_status.set(Some(msg));
            settings.refetch();
        }
    });

    // A live chip preview of the parsed permit set.
    let chips = Memo::new(move |_| {
        csv.get()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
    });

    view! {
        <div class="rounded-lg border border-border-primary bg-surface-secondary/40 p-5 space-y-3">
            <div>
                <h3 class="text-sm font-semibold text-text-primary">"Operator model access"</h3>
                <p class="text-[11.5px] text-text-muted mt-0.5 leading-snug max-w-xl">
                    "Comma-separated models an operator may pick in the chat model picker. \
                     Enforced server-side — an unlisted model is refused, not substituted. \
                     Leave empty to allow only the active default."
                </p>
            </div>
            <input type="text" placeholder="claude-sonnet-4-6, gpt-4o, llama3.2"
                class="w-full px-3 py-1.5 bg-surface-tertiary border border-border-primary rounded-md text-sm text-text-primary font-mono focus:outline-none focus:border-accent-amber"
                prop:value=move || csv.get()
                on:input=move |ev| csv.set(event_target_value(&ev)) />
            <div class="flex flex-wrap gap-1.5 min-h-[1.25rem]">
                {move || chips.get().into_iter().map(|c| view! {
                    <span class="text-[10.5px] font-mono px-2 py-0.5 rounded bg-surface-tertiary border border-border-primary text-text-secondary">{c}</span>
                }).collect_view()}
            </div>
            <div class="flex justify-end">
                <button
                    on:click=move |_| { save.dispatch(()); }
                    class="px-4 py-1.5 bg-surface-tertiary border border-border-primary text-text-primary font-medium rounded-md text-sm hover:border-accent-amber"
                >
                    "Save model access"
                </button>
            </div>
        </div>
    }
}
