//! Phase 7 — `/admin/observer` minimal viewer (anomalies + metric summary).

use leptos::prelude::*;

use crate::admin_observer::{list_anomalies, metric_summary, AnomalyRow, MetricSummaryRow};

#[component]
pub fn AdminObserver() -> impl IntoView {
    let anomalies = Resource::new(|| (), |_| list_anomalies(50));
    let metrics = Resource::new(|| (), |_| metric_summary(50));

    view! {
        <div class="flex flex-col h-full">
            <div class="px-4 py-3 border-b border-border-primary">
                <h1 class="text-lg font-semibold text-text-primary">"Observer"</h1>
                <div class="text-xs text-text-secondary">
                    "Phase 7 — telemetry from platform pollers + Prometheus ingest. "
                    "Set DAIMON_PROM_URL to enable Prometheus pulls."
                </div>
            </div>
            <div class="flex flex-1 overflow-hidden">
                <div class="w-1/2 overflow-y-auto p-4 border-r border-border-primary">
                    <h2 class="text-sm uppercase tracking-wider text-text-secondary mb-2">"Anomalies"</h2>
                    <Suspense fallback=|| view! { <div class="text-text-muted text-sm">"loading…"</div> }>
                        {move || anomalies.get().map(|res| match res {
                            Ok(rows) if rows.is_empty() => view! {
                                <div class="text-text-muted text-sm">"No anomalies recorded."</div>
                            }.into_any(),
                            Ok(rows) => view! {
                                <div class="space-y-2">
                                {rows.into_iter().map(|a: AnomalyRow| {
                                    let sev_class = match a.severity.as_str() {
                                        "critical" => "border-accent-danger/40 text-accent-danger",
                                        "error" => "border-accent-danger/40 text-accent-danger",
                                        "warning" => "border-yellow-500/40 text-yellow-300",
                                        _ => "border-border-primary text-text-muted",
                                    };
                                    view! {
                                        <div class=format!("rounded-md border {} bg-surface-secondary p-2", sev_class)>
                                            <div class="flex items-center justify-between">
                                                <div class="font-mono text-xs">{a.severity.clone()}</div>
                                                <div class="font-mono text-xs text-text-muted">{a.detected_at.clone()}</div>
                                            </div>
                                            <div class="text-sm text-text-primary">{a.title.clone()}</div>
                                            <div class="font-mono text-[11px] text-text-muted">
                                                {format!("{}/{}", a.source, a.source_id)}
                                            </div>
                                            {a.metric_name.as_ref().map(|m| view! {
                                                <div class="font-mono text-[11px] text-text-secondary">
                                                    {m.clone()}
                                                    {a.metric_value.map(|v| format!(" = {:.3}", v)).unwrap_or_default()}
                                                    {a.threshold.map(|t| format!(" (threshold {:.3})", t)).unwrap_or_default()}
                                                </div>
                                            })}
                                        </div>
                                    }
                                }).collect_view()}
                                </div>
                            }.into_any(),
                            Err(e) => view! { <div class="text-accent-danger text-sm">{format!("error: {e}")}</div> }.into_any(),
                        })}
                    </Suspense>
                </div>
                <div class="w-1/2 overflow-y-auto p-4">
                    <h2 class="text-sm uppercase tracking-wider text-text-secondary mb-2">"Metric Streams"</h2>
                    <Suspense fallback=|| view! { <div class="text-text-muted text-sm">"loading…"</div> }>
                        {move || metrics.get().map(|res| match res {
                            Ok(rows) if rows.is_empty() => view! {
                                <div class="text-text-muted text-sm">"No metrics recorded yet — platform poller writes here every 30s once a PVE cluster is registered."</div>
                            }.into_any(),
                            Ok(rows) => view! {
                                <table class="w-full text-sm">
                                    <thead>
                                        <tr class="text-text-secondary text-left">
                                            <th class="py-1 font-mono text-xs">"name"</th>
                                            <th class="py-1 font-mono text-xs">"source"</th>
                                            <th class="py-1 font-mono text-xs text-right">"value"</th>
                                            <th class="py-1 font-mono text-xs text-right">"samples"</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                    {rows.into_iter().map(|m: MetricSummaryRow| view! {
                                        <tr class="border-t border-border-primary">
                                            <td class="py-1 font-mono text-xs text-text-primary">{m.name}</td>
                                            <td class="py-1 font-mono text-[11px] text-text-muted">
                                                {format!("{}/{}", m.source, m.source_id)}
                                            </td>
                                            <td class="py-1 font-mono text-xs text-right">{format!("{:.3}", m.last_value)}</td>
                                            <td class="py-1 font-mono text-xs text-right text-text-muted">{m.sample_count}</td>
                                        </tr>
                                    }).collect_view()}
                                    </tbody>
                                </table>
                            }.into_any(),
                            Err(e) => view! { <div class="text-accent-danger text-sm">{format!("error: {e}")}</div> }.into_any(),
                        })}
                    </Suspense>
                </div>
            </div>
        </div>
    }
}
