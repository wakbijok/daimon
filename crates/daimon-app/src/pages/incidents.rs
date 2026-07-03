//! P7-9 (FR-UI-01/26) — the real Incidents surface.
//!
//! Detection → delivery in one place: the observer's anomalies (the incidents)
//! alongside the outbound alert deliveries the router recorded for them. Both
//! come from existing role-gated server-fns; the linked triage-plan and
//! recalled-prior-art views are a post-1.0 refinement (SHOULD tail of FR-UI-26).

use leptos::prelude::*;

use crate::admin_observer::{list_alert_deliveries, list_anomalies};

#[component]
pub fn Incidents() -> impl IntoView {
    let anomalies = Resource::new(|| (), |_| list_anomalies(100));
    let deliveries = Resource::new(|| (), |_| list_alert_deliveries(100));

    view! {
        <div class="space-y-6">
            <h1 class="text-xl font-semibold text-text-primary">"Incidents"</h1>

            <div>
                <h2 class="text-sm uppercase tracking-wider text-text-secondary mb-2">"Detected anomalies"</h2>
                <Suspense fallback=|| view! { <div class="text-text-muted text-sm">"loading…"</div> }>
                    {move || anomalies.get().map(|res| match res {
                        Ok(rows) if rows.is_empty() => view! {
                            <div class="text-text-muted text-sm py-4 text-center border border-border-primary rounded">
                                "No anomalies detected."
                            </div>
                        }.into_any(),
                        Ok(rows) => view! {
                            <div class="space-y-1">
                                {rows.into_iter().map(|a| {
                                    let sev_cls = sev_class(&a.severity);
                                    let resolved = a.resolved;
                                    view! {
                                        <div class="flex items-center gap-3 px-3 py-2 rounded border border-border-primary text-sm">
                                            <span class=format!("text-[10px] font-mono uppercase px-1.5 py-0.5 rounded {}", sev_cls)>{a.severity}</span>
                                            <span class="flex-1 min-w-0 truncate text-text-primary">{a.title}</span>
                                            <span class="text-text-muted font-mono text-xs truncate">{a.source_id}</span>
                                            {if resolved {
                                                view! { <span class="text-[10px] text-emerald-400">"resolved"</span> }.into_any()
                                            } else {
                                                view! { <span class="text-[10px] text-accent-amber">"open"</span> }.into_any()
                                            }}
                                            <span class="text-text-muted font-mono text-[10px]">{a.detected_at}</span>
                                        </div>
                                    }
                                }).collect_view()}
                            </div>
                        }.into_any(),
                        Err(e) => view! { <div class="text-accent-danger text-sm">{format!("error: {e}")}</div> }.into_any(),
                    })}
                </Suspense>
            </div>

            <div>
                <h2 class="text-sm uppercase tracking-wider text-text-secondary mb-2">"Alert deliveries"</h2>
                <Suspense fallback=|| view! { <div class="text-text-muted text-sm">"loading…"</div> }>
                    {move || deliveries.get().map(|res| match res {
                        Ok(rows) if rows.is_empty() => view! {
                            <div class="text-text-muted text-sm py-4 text-center border border-border-primary rounded">
                                "No alerts delivered yet."
                            </div>
                        }.into_any(),
                        Ok(rows) => view! {
                            <div class="space-y-1">
                                {rows.into_iter().map(|d| {
                                    let ok = d.status == "delivered";
                                    view! {
                                        <div class="flex items-center gap-3 px-3 py-2 rounded border border-border-primary text-sm">
                                            <span class="text-[10px] font-mono uppercase text-text-muted">{d.alert_class}</span>
                                            <span class="text-xs text-text-primary font-mono truncate">{d.channel}" → "{d.recipient}</span>
                                            <span class="flex-1"></span>
                                            {if ok {
                                                view! { <span class="text-[10px] text-emerald-400">"delivered"</span> }.into_any()
                                            } else {
                                                view! { <span class="text-[10px] text-accent-danger">"failed"</span> }.into_any()
                                            }}
                                            <span class="text-text-muted font-mono text-[10px]">{d.created_at}</span>
                                        </div>
                                    }
                                }).collect_view()}
                            </div>
                        }.into_any(),
                        Err(e) => view! { <div class="text-accent-danger text-sm">{format!("error: {e}")}</div> }.into_any(),
                    })}
                </Suspense>
            </div>
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
