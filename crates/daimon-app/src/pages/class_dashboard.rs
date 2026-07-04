//! UI-1 — per-class operate dashboards: Infrastructure (compute), Network,
//! Kubernetes (orchestrator). One shared component parameterised by target
//! class; UI-4 layers the visual telemetry on top. Registering/binding targets
//! is configuration → Settings (the rule), so this page only links there.

use leptos::prelude::*;

use crate::admin_targets::{list_targets, TargetKindDto};

#[component]
pub fn Infrastructure() -> impl IntoView {
    view! { <ClassDashboard title="Infrastructure" classes=vec![TargetKindDto::Host, TargetKindDto::App] blurb="hosts + apps — baremetal, VMs, mini-PCs" /> }
}

#[component]
pub fn NetworkDash() -> impl IntoView {
    view! { <ClassDashboard title="Network" classes=vec![TargetKindDto::Network] blurb="network + firewall targets" /> }
}

#[component]
pub fn KubernetesDash() -> impl IntoView {
    view! { <ClassDashboard title="Kubernetes" classes=vec![TargetKindDto::Platform] blurb="platform / orchestrator targets" /> }
}

#[component]
fn ClassDashboard(
    title: &'static str,
    classes: Vec<TargetKindDto>,
    blurb: &'static str,
) -> impl IntoView {
    let targets = Resource::new(|| (), |_| list_targets());
    let wanted = StoredValue::new(classes);

    view! {
        <div class="space-y-5">
            <div class="flex items-baseline gap-3">
                <h1 class="text-xl font-semibold text-text-primary">{title}</h1>
                <span class="text-xs font-mono text-text-muted">{blurb}</span>
                <a href="/settings" class="ml-auto text-xs px-3 py-1.5 rounded-md border border-border-primary text-text-secondary hover:text-accent-amber hover:border-accent-amber transition-colors">
                    "+ Register target → Settings"
                </a>
            </div>

            <Suspense fallback=|| view! { <div class="text-text-muted text-sm">"loading…"</div> }>
                {move || targets.get().map(|res| match res {
                    Ok(rows) => {
                        let mine: Vec<_> = rows
                            .into_iter()
                            .filter(|t| wanted.get_value().contains(&t.kind))
                            .collect();
                        if mine.is_empty() {
                            view! {
                                <div class="text-text-muted text-sm py-10 text-center border border-dashed border-border-primary rounded-xl">
                                    {format!("No {} targets registered yet. Register one in Settings → Connectors & Targets.", title.to_lowercase())}
                                </div>
                            }.into_any()
                        } else {
                            view! {
                                <div class="grid gap-3" style="grid-template-columns:repeat(auto-fill,minmax(280px,1fr))">
                                    {mine.into_iter().map(|t| {
                                        let transport = format!("{:?}", t.transport).to_lowercase();
                                        view! {
                                            <div class="rounded-xl border border-border-primary bg-surface-secondary p-4 space-y-3">
                                                <div class="flex items-center gap-3">
                                                    <div class="w-9 h-9 rounded-lg bg-surface-tertiary text-accent-amber flex items-center justify-center font-mono text-xs font-bold">
                                                        {title.chars().next().unwrap_or('T').to_string()}
                                                    </div>
                                                    <div class="min-w-0 flex-1">
                                                        <div class="text-sm font-semibold text-text-primary truncate">{t.ref_name.clone()}</div>
                                                        <div class="text-[10px] font-mono text-text-muted">{format!("{}:{} · {}", t.host, t.port, transport)}</div>
                                                    </div>
                                                    <span class="w-2 h-2 rounded-full bg-emerald-400 animate-pulse motion-reduce:animate-none"></span>
                                                </div>
                                                <div class="flex gap-4 text-[11px] font-mono text-text-muted">
                                                    <span>{format!("{} capabilities", t.capability_count)}</span>
                                                    <span>{format!("{} labels", t.label_count)}</span>
                                                </div>
                                            </div>
                                        }
                                    }).collect_view()}
                                </div>
                            }.into_any()
                        }
                    }
                    Err(e) => view! {
                        <div class="text-accent-danger text-sm">{format!("error: {e}")}</div>
                    }.into_any(),
                })}
            </Suspense>
        </div>
    }
}
