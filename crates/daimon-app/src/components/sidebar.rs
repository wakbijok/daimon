use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_location;
use super::icons::Icon;

struct NavItem {
    path: &'static str,
    label: &'static str,
    icon: &'static str,
}

struct ClusterItem {
    path: &'static str,
    label: &'static str,
}

const TOP_NAV: &[NavItem] = &[
    NavItem {
        path: "/",
        label: "Overview",
        icon: "M3.75 6A2.25 2.25 0 016 3.75h2.25A2.25 2.25 0 0110.5 6v2.25a2.25 2.25 0 01-2.25 2.25H6a2.25 2.25 0 01-2.25-2.25V6zM3.75 15.75A2.25 2.25 0 016 13.5h2.25a2.25 2.25 0 012.25 2.25V18a2.25 2.25 0 01-2.25 2.25H6A2.25 2.25 0 013.75 18v-2.25zM13.5 6a2.25 2.25 0 012.25-2.25H18A2.25 2.25 0 0120.25 6v2.25A2.25 2.25 0 0118 10.5h-2.25a2.25 2.25 0 01-2.25-2.25V6zM13.5 15.75a2.25 2.25 0 012.25-2.25H18a2.25 2.25 0 012.25 2.25V18A2.25 2.25 0 0118 20.25h-2.25A2.25 2.25 0 0113.5 18v-2.25z",
    },
    NavItem {
        path: "/incidents",
        label: "Incidents",
        icon: "M14.857 17.082a23.848 23.848 0 005.454-1.31A8.967 8.967 0 0118 9.75v-.7V9A6 6 0 006 9v.75a8.967 8.967 0 01-2.312 6.022c1.733.64 3.56 1.085 5.455 1.31m5.714 0a24.255 24.255 0 01-5.714 0m5.714 0a3 3 0 11-5.714 0",
    },
];

const CLUSTER_ITEMS: &[ClusterItem] = &[
    ClusterItem { path: "/cluster/nodes", label: "Nodes" },
    ClusterItem { path: "/cluster/vms", label: "VMs" },
    ClusterItem { path: "/cluster/containers", label: "Containers" },
    ClusterItem { path: "/cluster/storage", label: "Storage" },
];

#[component]
pub fn Sidebar() -> impl IntoView {
    let location = use_location();
    let pathname = move || location.pathname.get();
    let (collapsed, set_collapsed) = signal(false);
    let (cluster_expanded, set_cluster_expanded) = signal(true);

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
                    >
                        <Icon d="M3.75 6.75h16.5M3.75 12h16.5m-16.5 5.25H12".to_string() />
                    </button>
                </div>

                // Nav
                <nav class="flex-1 overflow-y-auto py-2 space-y-1">
                    // Top nav items
                    {TOP_NAV.iter().map(|item| {
                        let path = item.path;
                        let label = item.label;
                        let icon = item.icon.to_string();
                        view! {
                            <A
                                href=path
                                attr:class=move || format!(
                                    "flex items-center gap-2.5 mx-2 px-3 py-2 rounded-md text-[13px] font-medium transition-colors {}",
                                    if (path == "/" && pathname() == "/") || (path != "/" && pathname().starts_with(path)) {
                                        "text-text-primary bg-accent-amber/10"
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

                    // Divider
                    <div class="mx-4 border-t border-border-primary/50" />

                    // Cluster section
                    <div class="px-1">
                        <button
                            on:click=move |_| set_cluster_expanded.update(|e| *e = !*e)
                            class="w-full flex items-center gap-2 px-3 py-1.5 text-[11px] font-semibold uppercase tracking-wider text-text-muted hover:text-text-secondary transition-colors"
                        >
                            <svg
                                class=move || format!(
                                    "w-3.5 h-3.5 text-text-muted transition-transform duration-200 {}",
                                    if cluster_expanded.get() { "rotate-90" } else { "" }
                                )
                                fill="none" stroke="currentColor" viewBox="0 0 24 24"
                            >
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8.25 4.5l7.5 7.5-7.5 7.5" />
                            </svg>
                            <Show when=move || !collapsed.get()>
                                <span>"Cluster"</span>
                            </Show>
                        </button>

                        <Show when=move || cluster_expanded.get() && !collapsed.get()>
                            <div class="space-y-0.5 ml-2">
                                {CLUSTER_ITEMS.iter().map(|item| {
                                    let path = item.path;
                                    let label = item.label;
                                    view! {
                                        <A
                                            href=path
                                            attr:class=move || format!(
                                                "flex items-center gap-2.5 pl-6 pr-3 py-1 rounded-md text-[12px] transition-colors {}",
                                                if pathname().starts_with(path) {
                                                    "text-text-primary bg-accent-amber/10 border-l-2 border-accent-amber"
                                                } else {
                                                    "text-text-muted hover:text-text-secondary hover:bg-surface-tertiary"
                                                }
                                            )
                                        >
                                            <span class=move || format!(
                                                "w-1 h-1 rounded-full {}",
                                                if pathname().starts_with(path) { "bg-accent-amber" } else { "bg-text-muted/30" }
                                            ) />
                                            {label}
                                        </A>
                                    }
                                }).collect_view()}
                            </div>
                        </Show>
                    </div>
                </nav>

                // Bottom section
                <div class="border-t border-border-primary/50 py-2 space-y-1 shrink-0">
                    // Settings
                    <A
                        href="/settings"
                        attr:class=move || format!(
                            "flex items-center gap-2.5 mx-2 px-3 py-2 rounded-md text-[13px] font-medium transition-colors {}",
                            if pathname().starts_with("/settings") {
                                "text-text-primary bg-accent-amber/10"
                            } else {
                                "text-text-secondary hover:text-text-primary hover:bg-surface-tertiary"
                            }
                        )
                    >
                        <Icon d="M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.324.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 011.37.49l1.296 2.247a1.125 1.125 0 01-.26 1.431l-1.003.827c-.293.24-.438.613-.431.992a6.759 6.759 0 010 .255c-.007.378.138.75.43.99l1.005.828c.424.35.534.954.26 1.43l-1.298 2.247a1.125 1.125 0 01-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.57 6.57 0 01-.22.128c-.331.183-.581.495-.644.869l-.213 1.28c-.09.543-.56.941-1.11.941h-2.594c-.55 0-1.02-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 01-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 01-1.369-.49l-1.297-2.247a1.125 1.125 0 01.26-1.431l1.004-.827c.292-.24.437-.613.43-.992a6.932 6.932 0 010-.255c.007-.378-.138-.75-.43-.99l-1.004-.828a1.125 1.125 0 01-.26-1.43l1.297-2.247a1.125 1.125 0 011.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.087.22-.128.332-.183.582-.495.644-.869l.214-1.281z".to_string() />
                        <Show when=move || !collapsed.get()>
                            <span>"Settings"</span>
                        </Show>
                    </A>
                </div>
            </div>
        </aside>
    }
}
