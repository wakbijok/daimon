use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct MetricRow {
    pub name: String,
    pub sub: String,
    pub cpu_pct: f64,
    pub ram_pct: f64,
    pub status: String,
}

#[component]
pub fn MetricTable(rows: Vec<MetricRow>) -> impl IntoView {
    view! {
        <div class="overflow-x-auto">
            <table class="w-full text-sm text-left">
                <thead class="text-xs uppercase text-text-muted border-b border-border-primary">
                    <tr>
                        <th class="px-3 py-2">"Name"</th>
                        <th class="px-3 py-2">"CPU"</th>
                        <th class="px-3 py-2">"RAM"</th>
                        <th class="px-3 py-2">"Status"</th>
                    </tr>
                </thead>
                <tbody>
                    {rows.into_iter().map(|row| {
                        let name = row.name.clone();
                        let sub = row.sub.clone();
                        let cpu = row.cpu_pct;
                        let ram = row.ram_pct;
                        let status = row.status.clone();
                        view! {
                            <tr class="border-b border-border-primary/50 hover:bg-surface-tertiary/50">
                                <td class="px-3 py-2">
                                    <div class="text-text-primary font-medium">{name}</div>
                                    <div class="text-text-muted text-xs">{sub}</div>
                                </td>
                                <td class="px-3 py-2">
                                    <div class="flex items-center gap-2">
                                        <div class="w-16 h-1.5 bg-surface-tertiary rounded-full overflow-hidden">
                                            <div
                                                class="h-full bg-accent-amber rounded-full"
                                                style=format!("width: {}%", cpu.min(100.0))
                                            />
                                        </div>
                                        <span class="text-text-muted text-xs">{format!("{:.0}%", cpu)}</span>
                                    </div>
                                </td>
                                <td class="px-3 py-2">
                                    <div class="flex items-center gap-2">
                                        <div class="w-16 h-1.5 bg-surface-tertiary rounded-full overflow-hidden">
                                            <div
                                                class="h-full bg-blue-500 rounded-full"
                                                style=format!("width: {}%", ram.min(100.0))
                                            />
                                        </div>
                                        <span class="text-text-muted text-xs">{format!("{:.0}%", ram)}</span>
                                    </div>
                                </td>
                                <td class="px-3 py-2">
                                    <span class=format!(
                                        "inline-block w-2 h-2 rounded-full {}",
                                        if status == "running" || status == "online" { "bg-green-500" } else { "bg-text-muted/40" }
                                    ) />
                                </td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
}
