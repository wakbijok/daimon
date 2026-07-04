//! UI-1 — the operate-first sidebar (console v2, Wak's rule): the sidebar
//! carries the feature DASHBOARDS for using the platform. Configuration lives
//! in one place — Settings (bottom). The old Admin section is dissolved: its
//! operational pages are top-level dashboards now; its config pages moved into
//! Settings tabs.

use super::icons::Icon;
use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_location;

struct NavItem {
    path: &'static str,
    label: &'static str,
    icon: &'static str,
}

/// The Operate section — one entry per platform feature dashboard.
const OPERATE_NAV: &[NavItem] = &[
    NavItem {
        path: "/",
        label: "Overview",
        icon: "M3.75 6A2.25 2.25 0 016 3.75h2.25A2.25 2.25 0 0110.5 6v2.25a2.25 2.25 0 01-2.25 2.25H6a2.25 2.25 0 01-2.25-2.25V6zM3.75 15.75A2.25 2.25 0 016 13.5h2.25a2.25 2.25 0 012.25 2.25V18a2.25 2.25 0 01-2.25 2.25H6A2.25 2.25 0 013.75 18v-2.25zM13.5 6a2.25 2.25 0 012.25-2.25H18A2.25 2.25 0 0120.25 6v2.25A2.25 2.25 0 0118 10.5h-2.25a2.25 2.25 0 01-2.25-2.25V6zM13.5 15.75a2.25 2.25 0 012.25-2.25H18a2.25 2.25 0 012.25 2.25V18A2.25 2.25 0 0118 20.25h-2.25A2.25 2.25 0 0113.5 18v-2.25z",
    },
    NavItem {
        path: "/incidents",
        label: "Incidents",
        icon: "M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z",
    },
    NavItem {
        path: "/infrastructure",
        label: "Infrastructure",
        icon: "M5.25 14.25h13.5m-13.5 0a3 3 0 01-3-3m3 3a3 3 0 100 6h13.5a3 3 0 100-6m-16.5-3a3 3 0 013-3h13.5a3 3 0 013 3m-19.5 0a4.5 4.5 0 01.9-2.7L5.737 5.1a3.375 3.375 0 012.7-1.35h7.126c1.062 0 2.062.5 2.7 1.35l2.587 3.45a4.5 4.5 0 01.9 2.7m0 0a3 3 0 01-3 3m0 3h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008zm-3 6h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008z",
    },
    NavItem {
        path: "/network",
        label: "Network",
        icon: "M8.288 15.038a5.25 5.25 0 017.424 0M5.106 11.856c3.807-3.808 9.98-3.808 13.788 0M1.924 8.674c5.565-5.565 14.587-5.565 20.152 0M12.53 18.22l-.53.53-.53-.53a.75.75 0 011.06 0z",
    },
    NavItem {
        path: "/kubernetes",
        label: "Kubernetes",
        icon: "M21 7.5l-9-5.25L3 7.5m18 0l-9 5.25m9-5.25v9l-9 5.25M3 7.5l9 5.25M3 7.5v9l9 5.25m0-9v9",
    },
    NavItem {
        path: "/metrics",
        label: "Metrics",
        icon: "M2.25 18L9 11.25l4.306 4.307a11.95 11.95 0 015.814-5.519l2.74-1.22m0 0l-5.94-2.28m5.94 2.28l-2.28 5.941",
    },
    NavItem {
        path: "/topology",
        label: "Topology",
        icon: "M7.5 21L3 16.5m0 0L7.5 12M3 16.5h13.5m0-13.5L21 7.5m0 0L16.5 12M21 7.5H7.5",
    },
    NavItem {
        path: "/plans",
        label: "Plans",
        icon: "M9 12h3.75M9 15h3.75M9 18h3.75m3 .75H18a2.25 2.25 0 002.25-2.25V6.108c0-1.135-.845-2.098-1.976-2.192a48.424 48.424 0 00-1.123-.08m-5.801 0c-.065.21-.1.433-.1.664 0 .414.336.75.75.75h4.5a.75.75 0 00.75-.75 2.25 2.25 0 00-.1-.664m-5.8 0A2.251 2.251 0 0113.5 2.25H15c1.012 0 1.867.668 2.15 1.586m-5.8 0c-.376.023-.75.05-1.124.08C9.095 4.01 8.25 4.973 8.25 6.108V8.25m0 0H4.875c-.621 0-1.125.504-1.125 1.125v11.25c0 .621.504 1.125 1.125 1.125h9.75c.621 0 1.125-.504 1.125-1.125V9.375c0-.621-.504-1.125-1.125-1.125H8.25z",
    },
    NavItem {
        path: "/audit",
        label: "Audit Log",
        icon: "M3.75 6.75h16.5M3.75 12h16.5m-16.5 5.25H12",
    },
    NavItem {
        path: "/memory",
        label: "Memory",
        icon: "M12 3a4 4 0 00-4 4 4 4 0 00-3 6.6A4 4 0 007 21h10a4 4 0 002-7.4A4 4 0 0016 7a4 4 0 00-4-4z",
    },
];

#[component]
pub fn Sidebar() -> impl IntoView {
    let location = use_location();
    let pathname = move || location.pathname.get();
    let (collapsed, set_collapsed) = signal(false);

    view! {
        <aside class=move || format!(
            "hidden md:flex flex-col bg-surface-secondary border-r border-border-primary h-screen sticky top-0 transition-all duration-200 {}",
            if collapsed.get() { "w-14" } else { "w-56" }
        )>
            <div class="flex flex-col h-full">
                // Brand
                <div class="h-12 flex items-center justify-between px-3 border-b border-border-primary/50 shrink-0">
                    <Show when=move || !collapsed.get()>
                        <A href="/" attr:class="text-lg font-bold tracking-tight select-none">
                            <span class="text-text-primary">"dai"</span>
                            <span class="text-accent-amber">"mon"</span>
                        </A>
                    </Show>
                    <button
                        on:click=move |_| set_collapsed.update(|c| *c = !*c)
                        class="w-8 h-8 flex items-center justify-center rounded-md text-text-muted hover:text-text-primary hover:bg-surface-tertiary transition-colors"
                        aria-label="Toggle sidebar"
                    >
                        <Icon d="M3.75 6.75h16.5M3.75 12h16.5m-16.5 5.25H12".to_string() />
                    </button>
                </div>

                // Operate — the feature dashboards
                <nav class="flex-1 overflow-y-auto py-2 space-y-0.5">
                    <Show when=move || !collapsed.get()>
                        <div class="px-4 pt-1 pb-1.5 text-[10px] font-mono font-semibold uppercase tracking-[0.14em] text-text-muted/70">
                            "Operate"
                        </div>
                    </Show>
                    {OPERATE_NAV.iter().map(|item| {
                        let path = item.path;
                        let label = item.label;
                        let icon = item.icon.to_string();
                        view! {
                            <A
                                href=path
                                attr:class=move || format!(
                                    "flex items-center gap-2.5 mx-2 px-3 py-[7px] rounded-md text-[13px] font-medium transition-colors {}",
                                    if (path == "/" && pathname() == "/") || (path != "/" && pathname().starts_with(path)) {
                                        "text-accent-amber bg-accent-amber/10"
                                    } else {
                                        "text-text-secondary hover:text-text-primary hover:bg-surface-tertiary"
                                    }
                                )
                            >
                                <Icon d=icon.clone() />
                                <Show when=move || !collapsed.get()>
                                    <span>{label}</span>
                                </Show>
                            </A>
                        }
                    }).collect_view()}
                </nav>

                // System — configuration has ONE home
                <div class="border-t border-border-primary/50 py-2 shrink-0">
                    <Show when=move || !collapsed.get()>
                        <div class="px-4 pt-1 pb-1.5 text-[10px] font-mono font-semibold uppercase tracking-[0.14em] text-text-muted/70">
                            "System"
                        </div>
                    </Show>
                    <A
                        href="/settings"
                        attr:class=move || format!(
                            "flex items-center gap-2.5 mx-2 px-3 py-2 rounded-md text-[13px] font-medium transition-colors {}",
                            if pathname().starts_with("/settings") {
                                "text-accent-amber bg-accent-amber/10"
                            } else {
                                "text-text-secondary hover:text-text-primary hover:bg-surface-tertiary"
                            }
                        )
                    >
                        <Icon d="M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.324.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 011.37.49l1.296 2.247a1.125 1.125 0 01-.26 1.431l-1.003.827c-.293.24-.438.613-.431.992a6.759 6.759 0 010 .255c-.007.378.138.75.43.99l1.005.828c.424.35.534.954.26 1.43l-1.298 2.247a1.125 1.125 0 01-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.57 6.57 0 01-.22.128c-.331.183-.581.495-.644.869l-.213 1.28c-.09.543-.56.941-1.11.941h-2.594c-.55 0-1.02-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 01-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 01-1.369-.49l-1.297-2.247a1.125 1.125 0 01.26-1.431l1.004-.827c.292-.24.437-.613.43-.992a6.932 6.932 0 010-.255c.007-.378-.138-.75-.43-.99l-1.004-.828a1.125 1.125 0 01-.26-1.43l1.297-2.247a1.125 1.125 0 011.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.087.22-.128.332-.183.582-.495.644-.869l.214-1.281z".to_string() />
                        <Show when=move || !collapsed.get()>
                            <span class="flex-1">"Settings"</span>
                            <span class="text-[9px] font-mono uppercase px-1.5 py-0.5 rounded-full border border-accent-amber/40 text-accent-amber">"admin"</span>
                        </Show>
                    </A>
                </div>
            </div>
        </aside>
    }
}
