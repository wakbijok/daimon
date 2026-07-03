//! P7-8 (FR-UI-01/25) — the real Dashboard landing surface.
//!
//! A live status overview built by REUSING the existing role-gated server-fns
//! (observer anomalies, orchestrator plans) — it adds no new privileged data
//! path, it aggregates already-authenticated reads and renders them as a
//! functional card grid (real numbers over an empty shell).

use leptos::prelude::*;

use crate::admin_observer::list_anomalies;
use crate::admin_plans::list_plans;

#[component]
pub fn Dashboard() -> impl IntoView {
    let anomalies = Resource::new(|| (), |_| list_anomalies(50));
    let plans = Resource::new(|| (), |_| list_plans());

    let anomaly_count = Signal::derive(move || {
        anomalies
            .get()
            .map(|r| r.map(|v| v.len()).unwrap_or(0).to_string())
            .unwrap_or_else(|| "…".into())
    });
    let plan_count = Signal::derive(move || {
        plans
            .get()
            .map(|r| r.map(|v| v.len()).unwrap_or(0).to_string())
            .unwrap_or_else(|| "…".into())
    });
    let running_count = Signal::derive(move || {
        plans
            .get()
            .map(|r| {
                r.map(|v| v.iter().filter(|p| p.status == "running").count())
                    .unwrap_or(0)
                    .to_string()
            })
            .unwrap_or_else(|| "…".into())
    });
    let failed_count = Signal::derive(move || {
        plans
            .get()
            .map(|r| {
                r.map(|v| v.iter().filter(|p| p.status == "failed").count())
                    .unwrap_or(0)
                    .to_string()
            })
            .unwrap_or_else(|| "…".into())
    });

    view! {
        <div class="space-y-6">
            <h1 class="text-xl font-semibold text-text-primary">"Overview"</h1>

            <div class="grid grid-cols-2 md:grid-cols-4 gap-3">
                <StatCard label="Open anomalies".into() value=anomaly_count />
                <StatCard label="Plans".into() value=plan_count />
                <StatCard label="Running plans".into() value=running_count />
                <StatCard label="Failed plans".into() value=failed_count />
            </div>

            <div>
                <h2 class="text-sm uppercase tracking-wider text-text-secondary mb-2">"Recent anomalies"</h2>
                <Suspense fallback=|| view! { <div class="text-text-muted text-sm">"loading…"</div> }>
                    {move || anomalies.get().map(|res| match res {
                        Ok(rows) if rows.is_empty() => view! {
                            <div class="text-text-muted text-sm py-4 text-center border border-border-primary rounded">
                                "No anomalies detected."
                            </div>
                        }.into_any(),
                        Ok(rows) => view! {
                            <div class="space-y-1">
                                {rows.into_iter().take(8).map(|a| {
                                    let sev_cls = sev_class(&a.severity);
                                    view! {
                                    <a href="/incidents" class="flex items-center gap-3 px-3 py-2 rounded border border-border-primary hover:bg-surface-secondary text-sm">
                                        <span class=format!("text-[10px] font-mono uppercase px-1.5 py-0.5 rounded {}", sev_cls)>{a.severity}</span>
                                        <span class="flex-1 min-w-0 truncate text-text-primary">{a.title}</span>
                                        <span class="text-text-muted font-mono text-xs truncate">{a.source_id}</span>
                                        <span class="text-text-muted font-mono text-[10px]">{a.detected_at}</span>
                                    </a>
                                }}).collect_view()}
                            </div>
                        }.into_any(),
                        Err(e) => view! {
                            <div class="text-accent-danger text-sm">{format!("error: {e}")}</div>
                        }.into_any(),
                    })}
                </Suspense>
            </div>
        </div>
    }
}

#[component]
fn StatCard(label: String, value: Signal<String>) -> impl IntoView {
    view! {
        <div class="rounded-lg border border-border-primary bg-surface-secondary p-4">
            <div class="text-2xl font-semibold text-text-primary">{move || value.get()}</div>
            <div class="text-xs text-text-secondary mt-1">{label}</div>
        </div>
    }
}

fn sev_class(sev: &str) -> &'static str {
    match sev {
        "critical" => "bg-accent-danger/20 text-accent-danger",
        "warning" => "bg-accent-amber/20 text-accent-amber",
        _ => "bg-surface-tertiary text-text-muted",
    }
}
