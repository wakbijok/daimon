//! `/admin/memory` — Phase 3 #9 ingest + search UI for the long-term memory tier.
//!
//! Two tabs:
//! - **Ingest** — paste text + source ID + kind, click Ingest. Returns chunk count.
//! - **Search** — type a query, slide top-K, click Search. Shows score + source + snippet.
//!
//! Hardcoded tenant `"default"` for Phase 3; per-tenant scoping lands with Phase 2c
//! tenant/RBAC primitives.

use leptos::prelude::*;

use crate::admin_memory::{
    IngestRequest, IngestResult, SearchHit, SearchRequest, admin_memory_ingest, admin_memory_search,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Ingest,
    Search,
}

#[component]
pub fn AdminMemory() -> impl IntoView {
    let tab = RwSignal::new(Tab::Search);

    view! {
        <div class="space-y-6">
            <div>
                <h1 class="text-2xl font-semibold">Memory</h1>
                <p class="text-sm text-gray-500 mt-1">
                    "Long-term memory tier (Qdrant). Tenant = "
                    <code class="font-mono text-xs px-1 py-0.5 bg-gray-100 rounded">"default"</code>
                    " — per-tenant scope lands in Phase 2c."
                </p>
            </div>

            <div class="border-b border-gray-200">
                <nav class="flex space-x-4" aria-label="Tabs">
                    <TabButton tab tab_kind=Tab::Search label="Search" />
                    <TabButton tab tab_kind=Tab::Ingest label="Ingest" />
                </nav>
            </div>

            <Show
                when=move || tab.get() == Tab::Search
                fallback=move || view! { <IngestPane /> }
            >
                <SearchPane />
            </Show>
        </div>
    }
}

#[component]
fn TabButton(tab: RwSignal<Tab>, tab_kind: Tab, label: &'static str) -> impl IntoView {
    view! {
        <button
            class:border-b-2={move || tab.get() == tab_kind}
            class:border-blue-600={move || tab.get() == tab_kind}
            class:text-blue-700={move || tab.get() == tab_kind}
            class:text-gray-600={move || tab.get() != tab_kind}
            class="px-3 py-2 text-sm font-medium hover:text-gray-900"
            on:click=move |_| tab.set(tab_kind)
        >
            {label}
        </button>
    }
}

#[component]
fn SearchPane() -> impl IntoView {
    let query = RwSignal::new(String::new());
    let top_k = RwSignal::new(5u32);
    let last_query = RwSignal::new(None::<SearchRequest>);

    let results = Resource::new(
        move || last_query.get(),
        |maybe_req| async move {
            match maybe_req {
                None => Ok(Vec::<SearchHit>::new()),
                Some(req) => admin_memory_search(req).await,
            }
        },
    );

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let q = query.get();
        if q.trim().is_empty() {
            return;
        }
        last_query.set(Some(SearchRequest {
            query: q.trim().to_string(),
            top_k: top_k.get(),
        }));
    };

    view! {
        <div class="space-y-4">
            <form on:submit=on_submit class="space-y-3">
                <div>
                    <label class="block text-sm font-medium text-gray-700">Query</label>
                    <input
                        type="text"
                        prop:value=move || query.get()
                        on:input=move |ev| query.set(event_target_value(&ev))
                        placeholder="natural language question..."
                        class="mt-1 block w-full rounded border border-gray-300 px-3 py-2 text-sm shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                    />
                </div>
                <div class="flex items-center gap-4">
                    <label class="text-sm font-medium text-gray-700">Top K</label>
                    <input
                        type="range"
                        min="1"
                        max="25"
                        prop:value=move || top_k.get().to_string()
                        on:input=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<u32>() {
                                top_k.set(v);
                            }
                        }
                        class="flex-1 max-w-xs"
                    />
                    <span class="text-sm text-gray-700 w-8 text-right">{move || top_k.get()}</span>
                    <button
                        type="submit"
                        class="ml-auto rounded bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700"
                    >
                        Search
                    </button>
                </div>
            </form>

            <Suspense fallback=|| view! { <p class="text-sm text-gray-500">"loading..."</p> }>
                {move || results.get().map(|res| match res {
                    Ok(hits) if hits.is_empty() => view! {
                        <p class="text-sm text-gray-500">
                            {move || {
                                if last_query.get().is_some() {
                                    "no hits — collection empty or query unrelated."
                                } else {
                                    "type a query and click Search."
                                }
                            }}
                        </p>
                    }.into_any(),
                    Ok(hits) => view! {
                        <div class="overflow-x-auto">
                            <table class="min-w-full text-sm">
                                <thead class="bg-gray-50 text-left text-xs uppercase text-gray-500">
                                    <tr>
                                        <th class="px-3 py-2">"#"</th>
                                        <th class="px-3 py-2">Score</th>
                                        <th class="px-3 py-2">Source</th>
                                        <th class="px-3 py-2">Kind</th>
                                        <th class="px-3 py-2">Chunk</th>
                                        <th class="px-3 py-2">Snippet</th>
                                    </tr>
                                </thead>
                                <tbody class="divide-y divide-gray-200">
                                    {hits.into_iter().enumerate().map(|(i, h)| {
                                        let snippet = truncate(&h.text, 280);
                                        view! {
                                            <tr class="hover:bg-gray-50">
                                                <td class="px-3 py-2 text-gray-500">{i + 1}</td>
                                                <td class="px-3 py-2 font-mono">{format!("{:.4}", h.score)}</td>
                                                <td class="px-3 py-2 font-mono text-xs">{h.source_id}</td>
                                                <td class="px-3 py-2 text-xs">{h.source_kind}</td>
                                                <td class="px-3 py-2 text-xs text-gray-500">{h.chunk_index}</td>
                                                <td class="px-3 py-2 text-gray-700">{snippet}</td>
                                            </tr>
                                        }
                                    }).collect::<Vec<_>>()}
                                </tbody>
                            </table>
                        </div>
                    }.into_any(),
                    Err(e) => view! {
                        <p class="text-sm text-red-600">{format!("error: {}", e)}</p>
                    }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn IngestPane() -> impl IntoView {
    let source_id = RwSignal::new(String::new());
    let source_kind = RwSignal::new("doc".to_string());
    let content = RwSignal::new(String::new());
    let last_req = RwSignal::new(None::<IngestRequest>);

    let action = Resource::new(
        move || last_req.get(),
        |req| async move {
            match req {
                None => Ok(None),
                Some(r) => admin_memory_ingest(r).await.map(Some),
            }
        },
    );

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let sid = source_id.get();
        let kind = source_kind.get();
        let body = content.get();
        if sid.trim().is_empty() || body.trim().is_empty() {
            return;
        }
        last_req.set(Some(IngestRequest {
            source_id: sid.trim().to_string(),
            source_kind: kind.trim().to_string(),
            content: body,
        }));
    };

    view! {
        <form on:submit=on_submit class="space-y-3">
            <div class="grid grid-cols-2 gap-3">
                <div>
                    <label class="block text-sm font-medium text-gray-700">Source ID</label>
                    <input
                        type="text"
                        prop:value=move || source_id.get()
                        on:input=move |ev| source_id.set(event_target_value(&ev))
                        placeholder="stable id (e.g. faq-page-onboarding)"
                        class="mt-1 block w-full rounded border border-gray-300 px-3 py-2 text-sm shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                    />
                </div>
                <div>
                    <label class="block text-sm font-medium text-gray-700">Kind</label>
                    <input
                        type="text"
                        prop:value=move || source_kind.get()
                        on:input=move |ev| source_kind.set(event_target_value(&ev))
                        placeholder="doc | runbook | faq | code | spec"
                        class="mt-1 block w-full rounded border border-gray-300 px-3 py-2 text-sm shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                    />
                </div>
            </div>
            <div>
                <label class="block text-sm font-medium text-gray-700">Content</label>
                <textarea
                    rows="14"
                    prop:value=move || content.get()
                    on:input=move |ev| content.set(event_target_value(&ev))
                    placeholder="paste content to ingest..."
                    class="mt-1 block w-full rounded border border-gray-300 px-3 py-2 text-sm font-mono shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                ></textarea>
            </div>
            <div class="flex justify-end">
                <button
                    type="submit"
                    class="rounded bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700"
                >
                    Ingest
                </button>
            </div>

            <Suspense fallback=|| view! { <p class="text-sm text-gray-500">"working..."</p> }>
                {move || action.get().map(|res| match res {
                    Ok(None) => view! { <span /> }.into_any(),
                    Ok(Some(stats)) => view! {
                        <div class="rounded border border-green-200 bg-green-50 px-3 py-2 text-sm text-green-700">
                            "ingested " {stats.chunks} " chunks → collection " <code class="font-mono">{stats.collection}</code>
                        </div>
                    }.into_any(),
                    Err(e) => view! {
                        <div class="rounded border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
                            {format!("error: {}", e)}
                        </div>
                    }.into_any(),
                })}
            </Suspense>
        </form>
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

#[allow(dead_code)]
fn _stats_view_helper(stats: &IngestResult) -> String {
    format!("ingested {} chunks → {}", stats.chunks, stats.collection)
}
