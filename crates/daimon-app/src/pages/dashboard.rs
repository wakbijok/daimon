//! UI-4 (console v2) — the visual Overview: the operate-first landing surface.
//!
//! Wak's rule: "dashboard view yang memaparkan monitoring … tapi jangan heavy
//! dekat text, perlu lebih banyak visuals." Built by AGGREGATING the existing
//! role-gated reads (anomalies, plans, pending approvals, targets) — no new
//! privileged path — and rendering them as headline tiles, stacked distribution
//! bars, hand-rolled sparklines over real event timestamps, and a live
//! needs-attention feed with inline approve / deny. Every number is real; an
//! empty estate renders a calm "all clear", never a fake.

use leptos::prelude::*;

use crate::admin_approvals::{
    ApprovalRow, decide_approval, list_pending_approvals_with_blast_radius,
};
use crate::admin_observer::list_anomalies;
use crate::admin_plans::list_plans;
use crate::admin_targets::list_targets;
use crate::components::viz::{DistBar, DistSeg, StatTile, bucket_by_day, color, short_ts};

#[component]
pub fn Dashboard() -> impl IntoView {
    let anomalies = Resource::new(|| (), |_| list_anomalies(100));
    let plans = Resource::new(|| (), |_| list_plans());
    let targets = Resource::new(|| (), |_| list_targets());

    view! {
        <div class="space-y-6">
            <div>
                <h1 class="text-xl font-semibold text-text-primary">"Overview"</h1>
                <p class="text-[12px] text-text-muted mt-0.5">
                    "Operational status across the estate — live from the observer, orchestrator and approval queue."
                </p>
            </div>

            // ---- headline tiles (all reads under ONE Suspense: a Resource read
            //      outside a Suspense never resolves on hydrate — the v0.9.0 bug) --
            <Suspense fallback=|| view! { <TilesSkeleton /> }>
                {move || {
                    let a = anomalies.get();
                    let p = plans.get();
                    let t = targets.get();
                    match (a, p, t) {
                        (Some(a), Some(p), Some(t)) => {
                            let anomalies = a.unwrap_or_default();
                            let plans = p.unwrap_or_default();
                            let targets = t.unwrap_or_default();

                            let open: Vec<_> = anomalies.iter().filter(|x| !x.resolved).collect();
                            let open_count = open.len();
                            let crit = open.iter().filter(|x| x.severity == "critical").count();
                            let anomaly_spark = bucket_by_day(
                                &anomalies.iter().map(|x| x.detected_at.clone()).collect::<Vec<_>>(),
                                14,
                            );

                            let active = plans.iter().filter(|x| matches!(
                                x.status.as_str(), "planning" | "awaiting_approval" | "executing"
                            )).count();
                            let failed = plans.iter().filter(|x| x.status == "failed").count();
                            let plan_spark = bucket_by_day(
                                &plans.iter().map(|x| x.created_at.clone()).collect::<Vec<_>>(),
                                14,
                            );

                            let target_count = targets.len();

                            let open_cls = if crit > 0 { "text-accent-danger" } else if open_count > 0 { "text-accent-amber" } else { "text-text-primary" };
                            let failed_cls = if failed > 0 { "text-accent-danger" } else { "text-text-primary" };
                            let open_note = (crit > 0).then(|| format!("{crit} critical"));
                            let open_spark_color = (if crit > 0 { color::DANGER } else { color::AMBER }).to_string();
                            let active_note = format!("{} total", plans.len());

                            view! {
                                <div class="grid grid-cols-2 lg:grid-cols-4 gap-3">
                                    <StatTile
                                        label="Open anomalies"
                                        value=open_count.to_string()
                                        value_class=open_cls
                                        note=open_note
                                        spark=anomaly_spark
                                        spark_color=open_spark_color
                                    />
                                    <StatTile
                                        label="Active plans"
                                        value=active.to_string()
                                        note=Some(active_note)
                                        spark=plan_spark
                                        spark_color=color::GREEN.to_string()
                                    />
                                    <StatTile
                                        label="Failed plans"
                                        value=failed.to_string()
                                        value_class=failed_cls
                                    />
                                    <StatTile
                                        label="Registered targets"
                                        value=target_count.to_string()
                                        note=Some(String::from("hosts · network · platforms"))
                                    />
                                </div>

                                <div class="grid grid-cols-1 lg:grid-cols-3 gap-3">
                                    // needs-attention spans 2/3
                                    <div class="lg:col-span-2">
                                        <NeedsAttention />
                                    </div>
                                    // distributions stacked in the last column
                                    <div class="space-y-3">
                                        <DistBar title="Open anomaly severity" segments=severity_dist(&open) />
                                        <DistBar title="Plan outcomes" segments=plan_dist(&plans) />
                                    </div>
                                </div>

                                <RecentActivity anomalies=anomalies plans=plans />
                            }.into_any()
                        }
                        _ => view! { <TilesSkeleton /> }.into_any(),
                    }
                }}
            </Suspense>
        </div>
    }
}

/// Severity distribution over the OPEN anomalies (critical / warning / info).
fn severity_dist(open: &[&crate::admin_observer::AnomalyRow]) -> Vec<DistSeg> {
    let crit = open.iter().filter(|x| x.severity == "critical").count();
    let warn = open.iter().filter(|x| x.severity == "warning").count();
    let info = open.len().saturating_sub(crit + warn);
    vec![
        DistSeg {
            label: "Critical".into(),
            count: crit,
            color: color::DANGER.into(),
        },
        DistSeg {
            label: "Warning".into(),
            count: warn,
            color: color::AMBER.into(),
        },
        DistSeg {
            label: "Info".into(),
            count: info,
            color: color::MUTED.into(),
        },
    ]
}

/// Plan-outcome distribution across the real orchestrator status vocabulary.
/// An "Other" catch-all (shown only when non-zero) keeps the bar total equal to
/// the plan count even if a new `PlanStatus` variant is added upstream and not
/// mapped here — no plan silently vanishes from the chart.
fn plan_dist(plans: &[crate::admin_plans::PlanRow]) -> Vec<DistSeg> {
    let count = |f: &dyn Fn(&str) -> bool| plans.iter().filter(|p| f(&p.status)).count();
    let known = |s: &str| {
        matches!(
            s,
            "succeeded"
                | "planning"
                | "awaiting_approval"
                | "executing"
                | "failed"
                | "rolled_back"
                | "cancelled"
        )
    };
    let other = plans.iter().filter(|p| !known(&p.status)).count();
    let mut segs = vec![
        DistSeg {
            label: "Succeeded".into(),
            count: count(&|s| s == "succeeded"),
            color: color::GREEN.into(),
        },
        DistSeg {
            label: "Active".into(),
            count: count(&|s| matches!(s, "planning" | "awaiting_approval" | "executing")),
            color: color::AMBER.into(),
        },
        DistSeg {
            label: "Failed".into(),
            count: count(&|s| s == "failed"),
            color: color::DANGER.into(),
        },
        DistSeg {
            label: "Rolled back".into(),
            count: count(&|s| matches!(s, "rolled_back" | "cancelled")),
            color: color::PURPLE.into(),
        },
    ];
    if other > 0 {
        segs.push(DistSeg {
            label: "Other".into(),
            count: other,
            color: color::MUTED.into(),
        });
    }
    segs
}

#[component]
fn TilesSkeleton() -> impl IntoView {
    view! {
        <div class="grid grid-cols-2 lg:grid-cols-4 gap-3">
            {(0..4).map(|_| view! {
                <div class="rounded-xl border border-border-primary bg-surface-secondary p-4 h-24 animate-pulse motion-reduce:animate-none">
                    <div class="w-12 h-7 bg-surface-tertiary rounded"></div>
                    <div class="w-20 h-3 bg-surface-tertiary rounded mt-3"></div>
                </div>
            }).collect_view()}
        </div>
    }
}

// ---- Needs attention: pending approvals with inline approve / deny ----------

#[component]
fn NeedsAttention() -> impl IntoView {
    let pending = Resource::new(
        || (),
        |_| list_pending_approvals_with_blast_radius(Some(20)),
    );
    let (status, set_status) = signal::<Option<String>>(None);

    let decide = Action::new(move |args: &(String, bool)| {
        let (id, approved) = args.clone();
        async move {
            match decide_approval(id, approved).await {
                Ok(s) => Some(format!("decision recorded: {s}")),
                Err(e) => Some(format!("error: {e}")),
            }
        }
    });
    Effect::new(move |_| {
        if let Some(m) = decide.value().get().flatten() {
            set_status.set(Some(m));
            pending.refetch();
        }
    });

    view! {
        <div class="rounded-xl border border-border-primary bg-surface-secondary p-4 h-full flex flex-col">
            <div class="flex items-center justify-between mb-3">
                <h2 class="text-sm font-semibold text-text-primary">
                    "Needs attention"
                </h2>
                {move || status.get().map(|s| view! {
                    <span class="text-[11px] font-mono text-text-muted">{s}</span>
                })}
            </div>

            <Suspense fallback=|| view! { <div class="text-text-muted text-sm py-8 text-center">"loading…"</div> }>
                {move || pending.get().map(|res| match res {
                    // A non-admin (or graph-less) read: stay quiet, not scary.
                    Err(_) => view! {
                        <div class="text-text-muted text-sm py-8 text-center">
                            "Approval queue unavailable for this account."
                        </div>
                    }.into_any(),
                    Ok(rows) if rows.is_empty() => view! {
                        <div class="flex-1 flex flex-col items-center justify-center py-10 gap-2 text-center">
                            <div class="w-9 h-9 rounded-full bg-accent-green/15 text-accent-green flex items-center justify-center text-lg">"✓"</div>
                            <div class="text-sm text-text-secondary">"Nothing waiting."</div>
                            <div class="text-[11px] text-text-muted">"No gated action needs a decision right now."</div>
                        </div>
                    }.into_any(),
                    Ok(rows) => view! {
                        <div class="space-y-2">
                            {rows.into_iter().map(|r| view! {
                                <ApprovalCard row=r on_decide=Callback::new(move |(id, ok)| { decide.dispatch((id, ok)); }) />
                            }).collect_view()}
                        </div>
                    }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn ApprovalCard(
    row: ApprovalRow,
    #[prop(into)] on_decide: Callback<(String, bool)>,
) -> impl IntoView {
    let id_approve = row.id.clone();
    let id_deny = row.id.clone();
    let blast = row.blast_radius.clone();
    view! {
        <div class="rounded-lg border border-accent-amber/30 bg-accent-amber/5 p-3">
            <div class="flex items-start gap-3">
                <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2 flex-wrap">
                        <span class="text-sm font-semibold text-text-primary font-mono">{row.capability}</span>
                        {row.target_ref.clone().map(|t| view! {
                            <span class="text-[11px] font-mono text-accent-amber">{t}</span>
                        })}
                    </div>
                    <div class="text-[11px] text-text-muted mt-0.5">
                        "by " <span class="font-mono">{row.actor_id}</span>
                        " · " {short_ts(&row.created_at)}
                    </div>
                    {(!blast.is_empty()).then(|| {
                        let n = blast.len();
                        view! {
                            <div class="flex items-center gap-1.5 flex-wrap mt-2">
                                <span class="text-[10px] uppercase tracking-wider text-text-muted">{format!("blast · {n}")}</span>
                                {blast.into_iter().take(5).map(|b| view! {
                                    <span class="text-[10px] font-mono px-1.5 py-0.5 rounded bg-surface-tertiary border border-border-primary text-text-secondary">
                                        {format!("{}:{}", b.kind, b.label)}
                                    </span>
                                }).collect_view()}
                            </div>
                        }
                    })}
                </div>
                <div class="flex flex-col gap-1.5 shrink-0">
                    <button
                        on:click=move |_| on_decide.run((id_approve.clone(), true))
                        class="px-3 py-1 rounded-md bg-accent-green/90 hover:bg-accent-green text-surface-primary text-xs font-medium"
                    >"Approve"</button>
                    <button
                        on:click=move |_| on_decide.run((id_deny.clone(), false))
                        class="px-3 py-1 rounded-md bg-accent-danger/80 hover:bg-accent-danger text-text-primary text-xs font-medium"
                    >"Deny"</button>
                </div>
            </div>
        </div>
    }
}

// ---- Recent activity: merged anomalies + plans, newest first ----------------

#[component]
fn RecentActivity(
    anomalies: Vec<crate::admin_observer::AnomalyRow>,
    plans: Vec<crate::admin_plans::PlanRow>,
) -> impl IntoView {
    // Merge into a common shape (ts, kind, chip, chip_class, title), sort by the
    // RFC3339 timestamp descending (lexicographic = chronological), take 10.
    struct Ev {
        ts: String,
        kind: &'static str,
        chip: String,
        chip_class: &'static str,
        title: String,
        href: &'static str,
    }
    let mut evs: Vec<Ev> = Vec::new();
    for a in &anomalies {
        evs.push(Ev {
            ts: a.detected_at.clone(),
            kind: "anomaly",
            chip: a.severity.clone(),
            chip_class: match a.severity.as_str() {
                "critical" => "bg-accent-danger/20 text-accent-danger",
                "warning" => "bg-accent-amber/20 text-accent-amber",
                _ => "bg-surface-tertiary text-text-muted",
            },
            title: a.title.clone(),
            href: "/incidents",
        });
    }
    for p in &plans {
        evs.push(Ev {
            ts: p.created_at.clone(),
            kind: "plan",
            chip: p.status.clone(),
            chip_class: match p.status.as_str() {
                "succeeded" => "bg-accent-green/20 text-accent-green",
                "failed" => "bg-accent-danger/20 text-accent-danger",
                "rolled_back" | "cancelled" => "bg-accent-purple/20 text-accent-purple",
                _ => "bg-accent-amber/20 text-accent-amber",
            },
            title: p.intent.clone(),
            href: "/plans",
        });
    }
    evs.sort_by(|a, b| b.ts.cmp(&a.ts));

    let activity_spark = bucket_by_day(&evs.iter().map(|e| e.ts.clone()).collect::<Vec<_>>(), 21);

    view! {
        <div class="rounded-xl border border-border-primary bg-surface-secondary p-4">
            <div class="flex items-center justify-between mb-3">
                <h2 class="text-sm font-semibold text-text-primary">"Recent activity"</h2>
                <div class="opacity-70">
                    {(!activity_spark.is_empty()).then(|| view! {
                        <crate::components::sparkline::Sparkline data=activity_spark color=color::AMBER.to_string() width=120 height=24 fill=true />
                    })}
                </div>
            </div>
            {if evs.is_empty() {
                view! {
                    <div class="text-text-muted text-sm py-8 text-center">"No activity recorded yet."</div>
                }.into_any()
            } else {
                view! {
                    <div class="divide-y divide-border-primary/50">
                        {evs.into_iter().take(10).map(|e| view! {
                            <a href=e.href class="flex items-center gap-3 py-2 text-sm hover:bg-surface-tertiary/40 -mx-2 px-2 rounded">
                                <span class="text-[9.5px] uppercase tracking-wider text-text-muted w-14 shrink-0">{e.kind}</span>
                                <span class=format!("text-[10px] font-mono uppercase px-1.5 py-0.5 rounded shrink-0 {}", e.chip_class)>{e.chip}</span>
                                <span class="flex-1 min-w-0 truncate text-text-primary">{e.title}</span>
                                <span class="text-text-muted font-mono text-[10px] shrink-0">{short_ts(&e.ts)}</span>
                            </a>
                        }).collect_view()}
                    </div>
                }.into_any()
            }}
        </div>
    }
}
