//! Phase 8 — `/admin/approvals` operator inbox.
//!
//! Left pane: list of pending approval rows for the current tenant.
//! Right pane: detail for the selected row, including the NornicDB
//! blast-radius summary so the operator sees the impact before approving.

use leptos::prelude::*;

use crate::admin_approvals::{
    decide_approval, list_pending_approvals_with_blast_radius, ApprovalRow, BlastRadiusItem,
};

#[component]
pub fn AdminApprovals() -> impl IntoView {
    let approvals = Resource::new(|| (), |_| list_pending_approvals_with_blast_radius(Some(50)));
    let (selected_id, set_selected_id) = signal::<Option<String>>(None);
    let (status_msg, set_status_msg) = signal::<Option<String>>(None);

    let decide_action = Action::new(move |args: &(String, bool)| {
        let (id, approved) = args.clone();
        async move {
            match decide_approval(id, approved).await {
                Ok(s) => Some(format!("decision: {s}")),
                Err(e) => Some(format!("decide failed: {e}")),
            }
        }
    });

    Effect::new(move |_| {
        if let Some(m) = decide_action.value().get().flatten() {
            set_status_msg.set(Some(m));
            approvals.refetch();
            set_selected_id.set(None);
        }
    });

    let selected_row = Memo::new(move |_| {
        let id = selected_id.get()?;
        approvals
            .get()?
            .ok()?
            .into_iter()
            .find(|r| r.id == id)
    });

    view! {
        <div class="flex flex-col h-full">
            <div class="px-4 py-3 border-b border-border-primary flex items-center justify-between">
                <h1 class="text-lg font-semibold text-text-primary">"Approvals"</h1>
                {move || status_msg.get().map(|m| view! {
                    <div class="text-xs font-mono text-text-secondary">{m}</div>
                })}
            </div>
            <div class="flex flex-1 overflow-hidden">
                <div class="w-1/2 overflow-y-auto border-r border-border-primary p-4">
                    <h2 class="text-sm uppercase tracking-wider text-text-secondary mb-2">
                        "Pending"
                    </h2>
                    <Suspense fallback=|| view! {
                        <div class="text-text-secondary text-sm">"loading…"</div>
                    }>
                        {move || approvals.get().map(|res| match res {
                            Ok(rows) if rows.is_empty() => view! {
                                <div class="text-text-secondary text-sm py-6 text-center">
                                    "No pending approvals."
                                </div>
                            }.into_any(),
                            Ok(rows) => view! {
                                <table class="w-full text-sm">
                                    <thead>
                                        <tr class="text-text-secondary text-left">
                                            <th class="py-1">"Capability"</th>
                                            <th class="py-1">"Target"</th>
                                            <th class="py-1">"Actor"</th>
                                            <th class="py-1"></th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                    {rows.into_iter().map(|a| {
                                        let id_click = a.id.clone();
                                        let id_approve = a.id.clone();
                                        let id_deny = a.id.clone();
                                        let blast_count = a.blast_radius.len();
                                        view! {
                                            <tr class="border-t border-border-primary hover:bg-surface-secondary cursor-pointer"
                                                on:click=move |_| set_selected_id.set(Some(id_click.clone()))>
                                                <td class="py-2 text-text-primary font-mono text-xs">
                                                    {a.capability.clone()}
                                                    {(blast_count > 0).then(|| view! {
                                                        <span class="ml-1 text-[10px] text-accent-amber">
                                                            {format!("· {blast_count} impacted")}
                                                        </span>
                                                    })}
                                                </td>
                                                <td class="py-2 font-mono text-xs text-text-secondary">
                                                    {a.target_ref.clone().unwrap_or_default()}
                                                </td>
                                                <td class="py-2 font-mono text-xs text-text-secondary">
                                                    {a.actor_id.clone()}
                                                </td>
                                                <td class="py-2 text-right">
                                                    <button
                                                        on:click=move |ev| {
                                                            ev.stop_propagation();
                                                            decide_action.dispatch((id_approve.clone(), true));
                                                        }
                                                        class="px-2 py-1 bg-accent-amber text-surface-primary rounded text-xs mr-1"
                                                    >
                                                        "Approve"
                                                    </button>
                                                    <button
                                                        on:click=move |ev| {
                                                            ev.stop_propagation();
                                                            decide_action.dispatch((id_deny.clone(), false));
                                                        }
                                                        class="px-2 py-1 bg-accent-danger/80 text-text-primary rounded text-xs"
                                                    >
                                                        "Deny"
                                                    </button>
                                                </td>
                                            </tr>
                                        }
                                    }).collect_view()}
                                    </tbody>
                                </table>
                            }.into_any(),
                            Err(e) => view! {
                                <div class="text-accent-danger text-sm">
                                    {format!("error: {e}")}
                                </div>
                            }.into_any(),
                        })}
                    </Suspense>
                </div>
                <div class="w-1/2 overflow-y-auto p-4">
                    {move || match selected_row.get() {
                        None => view! {
                            <div class="text-text-secondary text-sm">"select a pending approval…"</div>
                        }.into_any(),
                        Some(a) => view! { <ApprovalDetail row=a /> }.into_any(),
                    }}
                </div>
            </div>
        </div>
    }
}

#[component]
fn ApprovalDetail(row: ApprovalRow) -> impl IntoView {
    let blast = row.blast_radius.clone();
    view! {
        <div class="space-y-3">
            <div>
                <div class="text-xs uppercase tracking-wider text-text-secondary">"Capability"</div>
                <div class="font-mono text-sm text-text-primary">{row.capability.clone()}</div>
            </div>
            {row.target_ref.clone().map(|t| view! {
                <div>
                    <div class="text-xs uppercase tracking-wider text-text-secondary">"Target"</div>
                    <div class="font-mono text-sm text-text-primary">{t}</div>
                </div>
            })}
            <div>
                <div class="text-xs uppercase tracking-wider text-text-secondary">"Actor"</div>
                <div class="font-mono text-sm text-text-primary">{row.actor_id.clone()}</div>
            </div>
            <div>
                <div class="text-xs uppercase tracking-wider text-text-secondary">"Created"</div>
                <div class="font-mono text-sm text-text-primary">{row.created_at.clone()}</div>
            </div>
            <div>
                <div class="text-xs uppercase tracking-wider text-text-secondary">"Params"</div>
                <pre class="mt-1 text-xs text-text-secondary whitespace-pre-wrap bg-surface-secondary p-2 rounded border border-border-primary">
                    {row.params_pretty.clone()}
                </pre>
            </div>
            <div>
                <div class="text-xs uppercase tracking-wider text-text-secondary mb-1">
                    {format!("Blast radius ({})", blast.len())}
                </div>
                {if blast.is_empty() {
                    view! {
                        <div class="text-text-secondary text-xs italic">
                            "no graph data — either graph tier disabled or no dependents reachable"
                        </div>
                    }.into_any()
                } else {
                    view! { <BlastRadiusTable items=blast /> }.into_any()
                }}
            </div>
        </div>
    }
}

#[component]
fn BlastRadiusTable(items: Vec<BlastRadiusItem>) -> impl IntoView {
    view! {
        <table class="w-full text-xs font-mono">
            <thead>
                <tr class="text-text-secondary text-left">
                    <th class="py-1">"Depth"</th>
                    <th class="py-1">"Kind"</th>
                    <th class="py-1">"Label"</th>
                </tr>
            </thead>
            <tbody>
                {items.into_iter().map(|i| view! {
                    <tr class="border-t border-border-primary">
                        <td class="py-1 text-accent-amber">{i.depth}</td>
                        <td class="py-1 text-text-secondary">{i.kind}</td>
                        <td class="py-1 text-text-primary">{i.label}</td>
                    </tr>
                }).collect_view()}
            </tbody>
        </table>
    }
}
