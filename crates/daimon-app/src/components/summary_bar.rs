use leptos::prelude::*;
use crate::components::sparkline::Sparkline;

pub struct SummaryItem {
    pub label: &'static str,
    pub value: String,
    pub color: Option<String>,
    pub sparkline_data: Option<Vec<f64>>,
    pub sparkline_color: Option<String>,
}

#[component]
pub fn SummaryBar(items: Vec<SummaryItem>) -> impl IntoView {
    view! {
        <div class="flex rounded-lg overflow-hidden bg-border-primary gap-px mb-4">
            {items.into_iter().map(|item| {
                let value_style = item.color
                    .map(|c| format!("color: {}", c))
                    .unwrap_or_default();
                let sparkline_color = item.sparkline_color
                    .unwrap_or_else(|| "#F59E0B".to_string());
                let sparkline_data = item.sparkline_data;

                view! {
                    <div class="flex-1 flex flex-col items-center justify-center gap-1 bg-surface-secondary px-3 py-2">
                        <span class="text-text-muted uppercase tracking-wide"
                            style="font-size: 10px;">
                            {item.label}
                        </span>
                        <span class="text-base font-bold text-text-primary"
                            style=value_style>
                            {item.value}
                        </span>
                        {sparkline_data.map(|data| view! {
                            <Sparkline
                                data=data
                                color=sparkline_color
                                width=80
                                height=20
                                fill=true
                            />
                        })}
                    </div>
                }
            }).collect_view()}
        </div>
    }
}
