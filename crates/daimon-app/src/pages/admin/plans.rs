//! Phase 6 D1 — `/admin/plans` minimal viewer.

use leptos::prelude::*;

use crate::admin_plans::{PlanRow, StepRow, get_plan_steps, list_plans, run_plan};

#[component]
pub fn AdminPlans() -> impl IntoView {
    let plans = Resource::new(|| (), |_| list_plans());
    let (selected, set_selected) = signal::<Option<String>>(None);
    let (status_msg, set_status_msg) = signal::<Option<String>>(None);

    let steps = Resource::new(
        move || selected.get(),
        |id| async move {
            if let Some(id) = id {
                get_plan_steps(id).await.unwrap_or_default()
            } else {
                Vec::new()
            }
        },
    );

    let run_action = Action::new(move |id: &String| {
        let id = id.clone();
        async move {
            match run_plan(id).await {
                Ok(s) => Some(format!("run complete: {s}")),
                Err(e) => Some(format!("run failed: {e}")),
            }
        }
    });

    Effect::new(move |_| {
        if let Some(m) = run_action.value().get().flatten() {
            set_status_msg.set(Some(m));
            plans.refetch();
            steps.refetch();
        }
    });

    view! {
        <div class="flex flex-col h-full">
            <div class="px-4 py-3 border-b border-border-primary flex items-center justify-between">
                <h1 class="text-lg font-semibold text-text-primary">"Plans"</h1>
                {move || status_msg.get().map(|m| view! {
                    <div class="text-xs font-mono text-text-secondary">{m}</div>
                })}
            </div>
            <div class="flex flex-1 overflow-hidden">
                <div class="w-1/2 overflow-y-auto border-r border-border-primary p-4">
                    <h2 class="text-sm uppercase tracking-wider text-text-secondary mb-2">"Recent"</h2>
                    <Suspense fallback=|| view! { <div class="text-text-secondary text-sm">"loading…"</div> }>
                        {move || plans.get().map(|res| match res {
                            Ok(rows) => view! {
                                <table class="w-full text-sm">
                                    <thead>
                                        <tr class="text-text-secondary text-left">
                                            <th class="py-1">"Intent"</th>
                                            <th class="py-1">"Status"</th>
                                            <th class="py-1"></th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                    {rows.into_iter().map(|p| {
                                        let id = p.id.clone();
                                        let id_for_run = id.clone();
                                        let id_for_click = id.clone();
                                        view! {
                                            <tr class="border-t border-border-primary hover:bg-surface-secondary cursor-pointer"
                                                on:click=move |_| set_selected.set(Some(id_for_click.clone()))>
                                                <td class="py-2 text-text-primary">{p.intent}</td>
                                                <td class="py-2 font-mono text-xs text-text-secondary">{p.status}</td>
                                                <td class="py-2 text-right">
                                                    <button
                                                        on:click=move |ev| {
                                                            ev.stop_propagation();
                                                            run_action.dispatch(id_for_run.clone());
                                                        }
                                                        class="px-2 py-1 bg-accent-amber text-surface-primary rounded text-xs"
                                                    >
                                                        "Run"
                                                    </button>
                                                </td>
                                            </tr>
                                        }
                                    }).collect_view()}
                                    </tbody>
                                </table>
                            }.into_any(),
                            Err(e) => view! { <div class="text-accent-danger text-sm">{format!("error: {e}")}</div> }.into_any(),
                        })}
                    </Suspense>
                </div>
                <div class="w-1/2 overflow-y-auto p-4">
                    <h2 class="text-sm uppercase tracking-wider text-text-secondary mb-2">"Steps"</h2>
                    <Suspense fallback=|| view! { <div class="text-text-secondary text-sm">"select a plan…"</div> }>
                        {move || steps.get().map(|rows: Vec<StepRow>| view! {
                            <div class="space-y-2">
                            {rows.into_iter().map(|s| view! {
                                <div class="rounded border border-border-primary p-2 bg-surface-secondary">
                                    <div class="flex items-center justify-between">
                                        <div class="font-mono text-xs text-text-secondary">{format!("step {}", s.step_index)}</div>
                                        <div class="font-mono text-xs text-accent-amber">{s.status}</div>
                                    </div>
                                    <div class="text-sm text-text-primary">{s.capability_name}</div>
                                    {s.target_ref.map(|t| view! { <div class="font-mono text-xs text-text-secondary">{t}</div> })}
                                    {s.result_summary.map(|r| view! {
                                        <pre class="mt-1 text-xs text-text-secondary whitespace-pre-wrap">{r}</pre>
                                    })}
                                </div>
                            }).collect_view()}
                            </div>
                        })}
                    </Suspense>
                </div>
            </div>
        </div>
    }
}

#[allow(dead_code)]
fn _unused(_: PlanRow) {}
