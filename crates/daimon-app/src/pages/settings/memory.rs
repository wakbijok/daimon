//! UI-7 — Memory settings, Hermes-style external-memory catalog (review round).
//!
//! Wak's reference: Hermes configures external memory as a provider catalog
//! (mem0 / honcho / holographic / …, ONE active at a time via
//! `memory.provider`) with a per-provider connection form. daimon rides on
//! dm-lite (the `dmem` sidecar) instead of baking a memory engine in — so
//! dm-lite is OUR entry in the listed set, first-class.
//!
//! STAGED, NOT WIRED (Wak: "jangan wire terus dm-lite kat situ — develop the
//! config menu je"): in this build the memory tier boots from the environment
//! (`DAIMON_DMEM_URL`) + the vault credential `dmem-bearer` (`main.rs` P3
//! wiring), falling back to NullMemory. The `memory.*` keys written here are
//! preserved in `app_config` but not yet consumed — they become the source of
//! truth in the VM-deployment arc, where each daimon VM ships its own dm-lite.
//! The banner below says exactly that, so no field silently pretends to be
//! live (the P6-6 honesty rule, kept by disclosure instead of read-only).

use leptos::prelude::*;
use serde_json::Value as Json;

use crate::admin_settings::{SettingRow, list_settings, set_setting};

fn row_string(rows: &[SettingRow], key: &str) -> String {
    rows.iter()
        .find(|r| r.key == key)
        .map(|r| match &r.value {
            Json::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

/// A memory backend in the catalog. `available` = selectable in THIS build
/// (config staged for deployment); false = shown greyed for roadmap context,
/// with no config panel — a visible non-claim, not a dead control.
#[derive(Clone, Copy)]
struct MemorySpec {
    id: &'static str,
    name: &'static str,
    tagline: &'static str,
    glyph: &'static str,
    accent: &'static str,
    available: bool,
}

const PROVIDERS: &[MemorySpec] = &[
    MemorySpec {
        id: "dmem",
        name: "dm-lite (dmem)",
        tagline: "Our memory engine — single-binary sidecar, bitemporal typed store, hybrid FTS5+vector recall, bearer-authenticated server mode.",
        glyph: "dm",
        accent: "text-accent-amber border-accent-amber/40",
        available: true,
    },
    MemorySpec {
        id: "none",
        name: "None (disabled)",
        tagline: "No long-term memory. Recall degrades gracefully (NullMemory) — chat still works, nothing is remembered across sessions.",
        glyph: "∅",
        accent: "text-text-secondary border-border-primary",
        available: true,
    },
    MemorySpec {
        id: "mem0",
        name: "Mem0",
        tagline: "Mem0 platform / OSS memory API.",
        glyph: "m0",
        accent: "text-text-muted border-border-primary",
        available: false,
    },
    MemorySpec {
        id: "honcho",
        name: "Honcho",
        tagline: "AI-native memory with dialectic recall.",
        glyph: "Ho",
        accent: "text-text-muted border-border-primary",
        available: false,
    },
];

#[component]
pub fn MemoryTab() -> impl IntoView {
    let settings = LocalResource::new(move || list_settings("memory.".to_string()));
    let (status, set_status) = signal::<Option<String>>(None);
    // Which provider's panel is open. Seeded from the stored memory.provider
    // once settings load; clicking an available card switches focus (saving in
    // the panel is what persists the choice).
    let focus: RwSignal<Option<&'static str>> = RwSignal::new(None);

    Effect::new(move |_| {
        if let Some(Ok(rows)) = settings.get() {
            if focus.get_untracked().is_none() {
                let stored = row_string(&rows, "memory.provider");
                let id = PROVIDERS
                    .iter()
                    .find(|p| p.available && p.id == stored)
                    .map(|p| p.id)
                    .unwrap_or("dmem");
                focus.set(Some(id));
            }
        }
    });

    view! {
        <div class="max-w-4xl space-y-6">
            <div class="flex items-start justify-between gap-4">
                <div>
                    <h2 class="text-lg font-semibold text-text-primary">"Memory"</h2>
                    <p class="text-[12px] text-text-muted mt-0.5 max-w-xl leading-snug">
                        "The long-term memory behind recall, triage context and lessons. daimon rides \
                         an external memory system — one provider active at a time."
                    </p>
                </div>
                {move || status.get().map(|s| view! {
                    <span class="text-xs font-mono text-text-secondary whitespace-nowrap pt-1">{s}</span>
                })}
            </div>

            // Honest staging banner — this menu is preparation for the VM
            // deployment arc; boot currently resolves memory from env + vault.
            <div class="rounded-md border border-accent-purple/30 bg-accent-purple/5 p-3 text-[12px] text-text-secondary leading-snug">
                <span class="font-semibold text-accent-purple">"Staged configuration. "</span>
                "In this build the memory tier connects at boot via "
                <code class="bg-surface-tertiary px-1 rounded">"DAIMON_DMEM_URL"</code>
                " + the vault credential "
                <code class="bg-surface-tertiary px-1 rounded">"dmem-bearer"</code>
                " (falling back to NullMemory if unset). Values saved here are preserved and become "
                "the source of truth in the VM deployment — each daimon VM ships with its own dm-lite."
            </div>

            <Suspense fallback=|| view! {
                <div class="text-text-muted text-sm py-8 text-center">"loading memory config…"</div>
            }>
                {move || settings.get().map(|res| match res {
                    Err(e) => view! {
                        <div class="text-accent-danger text-sm">{format!("error: {e}")}</div>
                    }.into_any(),
                    Ok(rows) => {
                        // The STORED active provider (what a deploy would use);
                        // `focus` is which panel is open in the UI right now.
                        let stored = {
                            let p = row_string(&rows, "memory.provider");
                            if p.is_empty() { "dmem".to_string() } else { p }
                        };
                        let rows_panel = rows;
                        view! {
                            <div class="grid grid-cols-1 sm:grid-cols-2 gap-3">
                                {PROVIDERS.iter().map(|spec| {
                                    let spec = *spec;
                                    let is_stored = stored == spec.id;
                                    view! { <MemoryCard spec=spec is_stored=is_stored focus=focus /> }
                                }).collect_view()}
                            </div>

                            {move || {
                                let rows = rows_panel.clone();
                                match focus.get() {
                                    Some("dmem") => view! {
                                        <DmemConfig rows=rows settings=settings set_status=set_status />
                                    }.into_any(),
                                    Some("none") => view! {
                                        <NoneConfig settings=settings set_status=set_status />
                                    }.into_any(),
                                    _ => view! { <span/> }.into_any(),
                                }
                            }}
                        }.into_any()
                    }
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn MemoryCard(
    spec: MemorySpec,
    is_stored: bool,
    focus: RwSignal<Option<&'static str>>,
) -> impl IntoView {
    let id = spec.id;
    let is_focused = Memo::new(move |_| focus.get() == Some(id));
    view! {
        <button
            disabled=!spec.available
            on:click=move |_| if spec.available { focus.set(Some(id)) }
            class=move || format!(
                "text-left rounded-lg border p-4 w-full transition-colors {}",
                if !spec.available {
                    "border-border-primary bg-surface-secondary opacity-55 cursor-not-allowed"
                } else if is_focused.get() {
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
                    <div class="flex items-center gap-2 flex-wrap">
                        <span class="text-sm font-semibold text-text-primary">{spec.name}</span>
                        {(is_stored && spec.available).then(|| view! {
                            <span class="text-[9.5px] font-semibold uppercase tracking-wider px-1.5 py-0.5 rounded bg-accent-amber text-surface-primary">
                                "Active (staged)"
                            </span>
                        })}
                        {(!spec.available).then(|| view! {
                            <span class="text-[9.5px] font-semibold uppercase tracking-wider px-1.5 py-0.5 rounded bg-surface-tertiary text-text-muted">
                                "Not in this build"
                            </span>
                        })}
                    </div>
                    <p class="text-[11.5px] text-text-muted mt-1 leading-snug">{spec.tagline}</p>
                </div>
            </div>
        </button>
    }
}

const INPUT_CLS: &str = "w-full px-3 py-1.5 bg-surface-tertiary border border-border-primary \
     rounded-md text-sm text-text-primary font-mono focus:outline-none focus:border-accent-amber";

/// One labelled field row (same layout as the channel cards).
#[component]
fn MemField(label: &'static str, help: &'static str, children: Children) -> impl IntoView {
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

#[component]
fn DmemConfig(
    rows: Vec<SettingRow>,
    settings: LocalResource<Result<Vec<SettingRow>, ServerFnError>>,
    set_status: WriteSignal<Option<String>>,
) -> impl IntoView {
    let seed_url = {
        let u = row_string(&rows, "memory.dmem.url");
        if u.is_empty() {
            "http://localhost:7071".to_string()
        } else {
            u
        }
    };
    let seed_cred = {
        let c = row_string(&rows, "memory.dmem.bearer_cred");
        if c.is_empty() {
            "dmem-bearer".to_string()
        } else {
            c
        }
    };

    let url = RwSignal::new(seed_url);
    let bearer_cred = RwSignal::new(seed_cred);

    let save = Action::new(move |_: &()| async move {
        let writes = [
            ("memory.provider", "dmem".to_string()),
            ("memory.dmem.url", url.get_untracked()),
            ("memory.dmem.bearer_cred", bearer_cred.get_untracked()),
        ];
        for (k, v) in writes {
            if v.trim().is_empty() {
                continue; // blank = unchanged
            }
            if let Err(e) = set_setting(k.to_string(), Json::String(v), false).await {
                return format!("error: {e}");
            }
        }
        "memory config staged ✓ (applies with the VM deployment wiring)".to_string()
    });
    Effect::new(move |_| {
        if let Some(msg) = save.value().get() {
            set_status.set(Some(msg));
            settings.refetch();
        }
    });

    view! {
        <div class="rounded-lg border border-border-primary bg-surface-secondary/60 p-5 space-y-4">
            <h3 class="text-sm font-semibold text-text-primary">"Connect dm-lite"</h3>

            <MemField label="Server URL" help="dm-lite server-mode endpoint (dmem serve). https:// for its built-in TLS; the deployment default is the VM-local sidecar.">
                <input type="text" placeholder="http://localhost:7071" class=INPUT_CLS
                    prop:value=move || url.get()
                    on:input=move |ev| url.set(event_target_value(&ev)) />
            </MemField>

            <MemField label="Bearer token credential" help="Name of the ApiToken credential holding the dmem bearer (create it under Credentials). The token itself never lives in app_config.">
                <input type="text" placeholder="dmem-bearer" class=INPUT_CLS
                    prop:value=move || bearer_cred.get()
                    on:input=move |ev| bearer_cred.set(event_target_value(&ev)) />
                <span class="inline-flex items-center gap-1.5 text-[10.5px] font-mono text-accent-green mt-1">
                    "🔒 resolved through the vault at boot — matches today's 'dmem-bearer' lookup"
                </span>
            </MemField>

            <div class="flex items-center gap-3 justify-end pt-1">
                <button
                    on:click=move |_| { save.dispatch(()); }
                    class="px-4 py-1.5 bg-accent-amber text-surface-primary font-medium rounded-md text-sm"
                >"Save memory config"</button>
            </div>
        </div>
    }
}

#[component]
fn NoneConfig(
    settings: LocalResource<Result<Vec<SettingRow>, ServerFnError>>,
    set_status: WriteSignal<Option<String>>,
) -> impl IntoView {
    let save = Action::new(move |_: &()| async move {
        match set_setting(
            "memory.provider".to_string(),
            Json::String("none".to_string()),
            false,
        )
        .await
        {
            Ok(_) => "memory disabled (staged) — recall will degrade to NullMemory".to_string(),
            Err(e) => format!("error: {e}"),
        }
    });
    Effect::new(move |_| {
        if let Some(msg) = save.value().get() {
            set_status.set(Some(msg));
            settings.refetch();
        }
    });
    view! {
        <div class="rounded-lg border border-border-primary bg-surface-secondary/60 p-5 space-y-3">
            <h3 class="text-sm font-semibold text-text-primary">"Disable long-term memory"</h3>
            <p class="text-[12px] text-text-muted leading-snug max-w-xl">
                "daimon runs without recall: chat and remediation still work, but nothing is \
                 remembered across sessions and triage loses historical context."
            </p>
            <div class="flex justify-end">
                <button
                    on:click=move |_| { save.dispatch(()); }
                    class="px-4 py-1.5 bg-surface-tertiary border border-border-primary text-text-primary font-medium rounded-md text-sm hover:border-accent-danger"
                >"Confirm — no memory"</button>
            </div>
        </div>
    }
}
