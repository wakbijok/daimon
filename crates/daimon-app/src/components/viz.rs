//! UI-4 — shared visual primitives for the operate dashboards (console v2).
//!
//! Hand-rolled, dependency-free, and SSR/hydrate-identical (same discipline as
//! `sparkline.rs`): a stat tile, a stacked distribution bar, and a timestamp
//! bucketer that turns a list of real event timestamps into a sparkline series —
//! so every visual is a rendering of real data, never a decorative fake.

use leptos::prelude::*;

use crate::components::sparkline::Sparkline;

/// Semantic colors (hex, for inline SVG/bar styling — Tailwind can't derive a
/// class from runtime data). Kept in sync with the theme tokens in tailwind.css.
pub mod color {
    pub const AMBER: &str = "#F59E0B";
    pub const GREEN: &str = "#4CAF50";
    pub const DANGER: &str = "#F44336";
    pub const PURPLE: &str = "#A78BFA";
    pub const MUTED: &str = "#6B7280";
}

/// Bucket RFC3339 timestamps into a per-day count series (oldest → newest),
/// keeping at most the last `max_buckets` active days. Pure string slicing on
/// the `YYYY-MM-DD` prefix — no `chrono` (which is ssr-only in this crate), so
/// it runs on the wasm client too. An honest "events per active day" trend.
pub fn bucket_by_day(timestamps: &[String], max_buckets: usize) -> Vec<f64> {
    use std::collections::BTreeMap;
    let mut per_day: BTreeMap<&str, f64> = BTreeMap::new();
    for ts in timestamps {
        if ts.len() >= 10 {
            *per_day.entry(&ts[..10]).or_insert(0.0) += 1.0;
        }
    }
    let counts: Vec<f64> = per_day.into_values().collect();
    if counts.len() > max_buckets {
        counts[counts.len() - max_buckets..].to_vec()
    } else {
        counts
    }
}

/// Shorten an RFC3339 timestamp to `MM-DD HH:MM` for compact display. Returns
/// the original string if it is too short to slice.
pub fn short_ts(rfc3339: &str) -> String {
    if rfc3339.len() >= 16 {
        format!("{} {}", &rfc3339[5..10], &rfc3339[11..16])
    } else {
        rfc3339.to_string()
    }
}

/// A headline metric tile: big number, label, optional note, optional trend
/// sparkline. `value_class` carries the semantic accent (e.g. amber when a count
/// needs attention).
#[component]
pub fn StatTile(
    label: &'static str,
    #[prop(into)] value: String,
    #[prop(default = None)] note: Option<String>,
    #[prop(default = "text-text-primary")] value_class: &'static str,
    #[prop(optional)] spark: Option<Vec<f64>>,
    #[prop(default = String::from(color::AMBER))] spark_color: String,
) -> impl IntoView {
    view! {
        <div class="rounded-xl border border-border-primary bg-surface-secondary p-4 flex flex-col gap-2">
            <div class="flex items-start justify-between gap-2">
                <div class=format!("text-3xl font-semibold tabular-nums leading-none {value_class}")>
                    {value}
                </div>
                {spark.filter(|s| !s.is_empty()).map(|data| view! {
                    <div class="opacity-80 shrink-0">
                        <Sparkline data=data color=spark_color width=72 height=26 fill=true />
                    </div>
                })}
            </div>
            <div class="text-xs text-text-secondary">{label}</div>
            {note.map(|n| view! { <div class="text-[10.5px] text-text-muted">{n}</div> })}
        </div>
    }
}

/// One slice of a distribution.
#[derive(Clone)]
pub struct DistSeg {
    pub label: String,
    pub count: usize,
    /// Hex color for the segment + legend dot.
    pub color: String,
}

/// A stacked horizontal distribution bar + legend. Reads well at any total
/// (percentages), and renders a calm empty baseline when there's no data.
#[component]
pub fn DistBar(title: &'static str, segments: Vec<DistSeg>) -> impl IntoView {
    let total: usize = segments.iter().map(|s| s.count).sum();
    view! {
        <div class="rounded-xl border border-border-primary bg-surface-secondary p-4 space-y-3">
            <div class="flex items-baseline justify-between">
                <h3 class="text-xs uppercase tracking-wider text-text-secondary">{title}</h3>
                <span class="text-xs font-mono text-text-muted tabular-nums">{total.to_string()}</span>
            </div>
            {if total == 0 {
                view! {
                    <div class="h-2 rounded-full bg-surface-tertiary"></div>
                    <div class="text-[11px] text-text-muted">"no data yet"</div>
                }.into_any()
            } else {
                let segs_bar = segments.clone();
                view! {
                    <div class="flex h-2 rounded-full overflow-hidden bg-surface-tertiary">
                        {segs_bar.into_iter().filter(|s| s.count > 0).map(|s| {
                            let pct = (s.count as f64 / total as f64) * 100.0;
                            view! {
                                <div
                                    style=format!("width:{pct:.1}%;background-color:{}", s.color)
                                    title=format!("{}: {}", s.label, s.count)
                                ></div>
                            }
                        }).collect_view()}
                    </div>
                    <div class="flex flex-wrap gap-x-4 gap-y-1">
                        {segments.into_iter().map(|s| view! {
                            <span class="inline-flex items-center gap-1.5 text-[11px] text-text-secondary">
                                <span class="w-2 h-2 rounded-full shrink-0" style=format!("background-color:{}", s.color)></span>
                                {s.label}
                                <span class="font-mono text-text-muted tabular-nums">{s.count.to_string()}</span>
                            </span>
                        }).collect_view()}
                    </div>
                }.into_any()
            }}
        </div>
    }
}
