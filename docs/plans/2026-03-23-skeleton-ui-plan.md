# Skeleton UI Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the daimon full-stack Rust skeleton — login, sidebar navigation, empty pages — using Leptos 0.8 + Axum + SQLite with DataGate's dark+amber theme.

**Architecture:** Leptos SSR with WASM hydration. cargo-leptos builds both server binary and WASM client. Axum serves the app. SQLite stores users/sessions. JWT in HttpOnly cookies for auth. Tailwind 4 for styling.

**Tech Stack:** Leptos 0.8, Axum 0.8, leptos_axum 0.8, rusqlite (bundled), jsonwebtoken, bcrypt, Tailwind CSS 4

**Spec:** `docs/specs/2026-03-23-skeleton-ui-design.md`

**Note:** Spec says Leptos 0.7 but latest stable is 0.8.17. Plan uses 0.8.

---

## Chunk 1: Project Setup (cargo-leptos + daimon-app crate)

### Task 1: Install cargo-leptos and add WASM target

**Files:** None (toolchain setup)

- [ ] **Step 1: Install cargo-leptos**

```bash
cargo install cargo-leptos --locked
```

- [ ] **Step 2: Add WASM compilation target**

```bash
rustup target add wasm32-unknown-unknown
```

- [ ] **Step 3: Verify both installed**

```bash
cargo leptos --version
rustup target list --installed | grep wasm32
```

### Task 2: Remove daimon-bin, create daimon-app crate

**Files:**
- Delete: `crates/daimon-bin/` (entire directory)
- Create: `crates/daimon-app/Cargo.toml`
- Create: `crates/daimon-app/src/main.rs`
- Create: `crates/daimon-app/src/lib.rs`
- Create: `crates/daimon-app/src/app.rs`
- Modify: `Cargo.toml` (workspace root — add leptos metadata)

- [ ] **Step 1: Remove daimon-bin**

```bash
rm -rf crates/daimon-bin
```

- [ ] **Step 2: Create daimon-app Cargo.toml**

Create `crates/daimon-app/Cargo.toml`:

```toml
[package]
name = "daimon-app"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
description = "daimon web application — Leptos full-stack"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
leptos = { version = "0.8" }
leptos_router = { version = "0.8" }
leptos_meta = { version = "0.8" }
serde = { version = "1", features = ["derive"] }
console_error_panic_hook = { version = "0.1", optional = true }
wasm-bindgen = { version = "0.2", optional = true }

# Server-only dependencies
axum = { version = "0.8", optional = true }
tokio = { version = "1", features = ["rt-multi-thread"], optional = true }
leptos_axum = { version = "0.8", optional = true }
rusqlite = { version = "0.34", features = ["bundled"], optional = true }
jsonwebtoken = { version = "9", optional = true }
bcrypt = { version = "0.17", optional = true }
rand = { version = "0.9", optional = true }

[features]
hydrate = [
    "leptos/hydrate",
    "leptos_router/hydrate",
    "dep:console_error_panic_hook",
    "dep:wasm-bindgen",
]
ssr = [
    "dep:axum",
    "dep:tokio",
    "dep:leptos_axum",
    "leptos/ssr",
    "leptos_router/ssr",
    "dep:rusqlite",
    "dep:jsonwebtoken",
    "dep:bcrypt",
    "dep:rand",
]
```

- [ ] **Step 3: Update workspace root Cargo.toml**

Replace the existing workspace `Cargo.toml` with:

```toml
[workspace]
members = ["crates/*"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT"
repository = "https://github.com/wakbijok/daimon"

[profile.wasm-release]
inherits = "release"
opt-level = 'z'
lto = true
codegen-units = 1
panic = "abort"

[[workspace.metadata.leptos]]
name = "daimon-app"
output-name = "daimon"
site-root = "target/site"
site-pkg-dir = "pkg"
tailwind-input-file = "crates/daimon-app/style/tailwind.css"
tailwind-config-file = ""
assets-dir = "crates/daimon-app/public"
site-addr = "127.0.0.1:3000"
reload-port = 3001
bin-features = ["ssr"]
bin-default-features = false
lib-features = ["hydrate"]
lib-default-features = false
lib-profile-release = "wasm-release"
env = "DEV"
```

- [ ] **Step 4: Create minimal main.rs (server entrypoint)**

Create `crates/daimon-app/src/main.rs`:

```rust
#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::Router;
    use leptos::logging::log;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use daimon_app::app::*;

    let conf = get_configuration(None).unwrap();
    let addr = conf.leptos_options.site_addr;
    let leptos_options = conf.leptos_options;
    let routes = generate_route_list(App);

    let app = Router::new()
        .leptos_routes(&leptos_options, routes, {
            let leptos_options = leptos_options.clone();
            move || shell(leptos_options.clone())
        })
        .fallback(leptos_axum::file_and_error_handler(shell))
        .with_state(leptos_options);

    log!("daimon listening on http://{}", &addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

#[cfg(not(feature = "ssr"))]
pub fn main() {
    // Client-side: no main needed, hydration in lib.rs
}
```

- [ ] **Step 5: Create lib.rs (hydration entry)**

Create `crates/daimon-app/src/lib.rs`:

```rust
pub mod app;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use app::*;
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
}
```

- [ ] **Step 6: Create minimal app.rs (root component)**

Create `crates/daimon-app/src/app.rs`:

```rust
use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes},
    StaticSegment,
};

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone() />
                <HydrationScripts options/>
                <MetaTags/>
            </head>
            <body class="bg-[#080C14] text-gray-200">
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/daimon.css"/>
        <Title text="daimon"/>
        <Router>
            <main class="p-8">
                <Routes fallback=|| "Page not found.".into_view()>
                    <Route path=StaticSegment("") view=HomePage/>
                </Routes>
            </main>
        </Router>
    }
}

#[component]
fn HomePage() -> impl IntoView {
    view! {
        <h1 class="text-2xl font-bold">"daimon"</h1>
        <p class="text-gray-400 mt-2">"AI-driven system engineer for Proxmox"</p>
    }
}
```

- [ ] **Step 7: Create Tailwind CSS entry**

Create `crates/daimon-app/style/tailwind.css`:

```css
@import "tailwindcss";
@source "../src/**/*.rs";
```

- [ ] **Step 8: Create public directory**

```bash
mkdir -p crates/daimon-app/public
touch crates/daimon-app/public/.gitkeep
```

- [ ] **Step 9: Update .gitignore for Leptos artifacts**

Append to `.gitignore`:

```
# Leptos build artifacts
target/site/
```

- [ ] **Step 10: Verify it builds and runs**

```bash
cargo leptos watch
```

Open `http://127.0.0.1:3000` in browser. Expect: dark page with "daimon" heading.

Press Ctrl+C to stop.

- [ ] **Step 11: Commit**

```bash
git add -A
git commit -m "Replace daimon-bin with daimon-app (Leptos 0.8 + Axum full-stack)"
```

---

## Chunk 2: Theme + Layout Shell

### Task 3: Port DataGate theme to CSS variables

**Files:**
- Modify: `crates/daimon-app/style/tailwind.css`

- [ ] **Step 1: Add DataGate CSS variables and Tailwind theme**

Replace `crates/daimon-app/style/tailwind.css` with:

```css
@import "tailwindcss";
@source "../src/**/*.rs";

@theme {
    /* Surface colors */
    --color-surface-primary: #080C14;
    --color-surface-secondary: #0D1117;
    --color-surface-tertiary: #161B22;

    /* Text colors */
    --color-text-primary: #E6EDF3;
    --color-text-secondary: #9CA3AF;
    --color-text-muted: #6B7280;

    /* Border */
    --color-border-primary: #21262D;

    /* Accent colors */
    --color-accent-amber: #F59E0B;
    --color-accent-green: #4CAF50;
    --color-accent-danger: #F44336;
    --color-accent-purple: #A78BFA;
}
```

- [ ] **Step 2: Verify theme loads**

```bash
cargo leptos watch
```

Open browser, inspect body — should have `#080C14` background.

- [ ] **Step 3: Commit**

```bash
git add crates/daimon-app/style/tailwind.css
git commit -m "Add DataGate dark+amber theme (CSS variables via Tailwind 4)"
```

### Task 4: Create layout and sidebar components

**Files:**
- Create: `crates/daimon-app/src/components/mod.rs`
- Create: `crates/daimon-app/src/components/layout.rs`
- Create: `crates/daimon-app/src/components/sidebar.rs`
- Create: `crates/daimon-app/src/components/icons.rs`
- Modify: `crates/daimon-app/src/lib.rs` (add components module)
- Modify: `crates/daimon-app/src/app.rs` (use Layout)

- [ ] **Step 1: Create icons.rs**

Create `crates/daimon-app/src/components/icons.rs`:

```rust
use leptos::prelude::*;

/// SVG icon from a heroicons path string
#[component]
pub fn Icon(
    #[prop(into)] d: String,
    #[prop(default = "w-4 h-4".to_string(), into)] class: String,
) -> impl IntoView {
    view! {
        <svg class=class fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d=d />
        </svg>
    }
}

/// Chevron for collapsible sections
#[component]
pub fn Chevron(expanded: ReadSignal<bool>) -> impl IntoView {
    view! {
        <svg
            class=move || format!(
                "w-3.5 h-3.5 text-text-muted transition-transform duration-200 {}",
                if expanded.get() { "rotate-90" } else { "" }
            )
            fill="none" stroke="currentColor" viewBox="0 0 24 24"
        >
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8.25 4.5l7.5 7.5-7.5 7.5" />
        </svg>
    }
}
```

- [ ] **Step 2: Create sidebar.rs**

Create `crates/daimon-app/src/components/sidebar.rs`:

```rust
use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_location;
use super::icons::Icon;

/// Navigation item definition
struct NavItem {
    path: &'static str,
    label: &'static str,
    icon: &'static str,
}

/// Cluster sub-item
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

// No placeholder nav items — Monitoring is built into each page, AI is a floating chatbot (future)

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
```

- [ ] **Step 3: Create layout.rs**

Create `crates/daimon-app/src/components/layout.rs`:

```rust
use leptos::prelude::*;
use leptos_router::components::Outlet;
use super::sidebar::Sidebar;

#[component]
pub fn Layout() -> impl IntoView {
    view! {
        <div class="flex min-h-screen bg-surface-primary text-text-primary">
            <Sidebar />
            <main class="flex-1 min-w-0 p-4 sm:p-6">
                <div class="max-w-[1400px] mx-auto">
                    <Outlet />
                </div>
            </main>
        </div>
    }
}
```

- [ ] **Step 4: Create components/mod.rs**

Create `crates/daimon-app/src/components/mod.rs`:

```rust
pub mod icons;
pub mod layout;
pub mod sidebar;
```

- [ ] **Step 5: Add components module to lib.rs**

Update `crates/daimon-app/src/lib.rs` to:

```rust
pub mod app;
pub mod components;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use app::*;
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
}
```

- [ ] **Step 6: Verify layout renders**

```bash
cargo leptos watch
```

Open browser — should see dark sidebar with "daimon" brand, nav items, and main content area.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "Add layout shell — sidebar navigation with DataGate theme"
```

---

## Chunk 3: Routing + Placeholder Pages

### Task 5: Create placeholder page components

**Files:**
- Create: `crates/daimon-app/src/pages/mod.rs`
- Create: `crates/daimon-app/src/pages/dashboard.rs`
- Create: `crates/daimon-app/src/pages/incidents.rs`
- Create: `crates/daimon-app/src/pages/cluster/mod.rs`
- Create: `crates/daimon-app/src/pages/cluster/nodes.rs`
- Create: `crates/daimon-app/src/pages/cluster/vms.rs`
- Create: `crates/daimon-app/src/pages/cluster/containers.rs`
- Create: `crates/daimon-app/src/pages/cluster/storage.rs`
- Create: `crates/daimon-app/src/pages/incident_detail.rs`
- Create: `crates/daimon-app/src/pages/settings.rs`
- Create: `crates/daimon-app/src/pages/login.rs`
- Modify: `crates/daimon-app/src/lib.rs` (add pages module)

- [ ] **Step 1: Create page template macro and individual pages**

Each placeholder page follows the same pattern. Create `crates/daimon-app/src/pages/mod.rs`:

```rust
pub mod login;
pub mod dashboard;
pub mod incidents;
pub mod incident_detail;
pub mod cluster;
pub mod settings;
```

Create `crates/daimon-app/src/pages/dashboard.rs`:

```rust
use leptos::prelude::*;

#[component]
pub fn Dashboard() -> impl IntoView {
    view! {
        <div>
            <h1 class="text-xl font-semibold text-text-primary">"Overview"</h1>
            <p class="text-text-muted mt-2 text-sm">"Cluster dashboard — coming soon"</p>
        </div>
    }
}
```

Create `crates/daimon-app/src/pages/incidents.rs`:

```rust
use leptos::prelude::*;

#[component]
pub fn Incidents() -> impl IntoView {
    view! {
        <div>
            <h1 class="text-xl font-semibold text-text-primary">"Incidents"</h1>
            <p class="text-text-muted mt-2 text-sm">"Incident management — coming soon"</p>
        </div>
    }
}
```

Create `crates/daimon-app/src/pages/cluster/mod.rs`:

```rust
pub mod nodes;
pub mod vms;
pub mod containers;
pub mod storage;
```

Create `crates/daimon-app/src/pages/cluster/nodes.rs`:

```rust
use leptos::prelude::*;

#[component]
pub fn Nodes() -> impl IntoView {
    view! {
        <div>
            <h1 class="text-xl font-semibold text-text-primary">"Nodes"</h1>
            <p class="text-text-muted mt-2 text-sm">"Proxmox VE node management — coming soon"</p>
        </div>
    }
}
```

Create `crates/daimon-app/src/pages/cluster/vms.rs`:

```rust
use leptos::prelude::*;

#[component]
pub fn Vms() -> impl IntoView {
    view! {
        <div>
            <h1 class="text-xl font-semibold text-text-primary">"Virtual Machines"</h1>
            <p class="text-text-muted mt-2 text-sm">"VM management — coming soon"</p>
        </div>
    }
}
```

Create `crates/daimon-app/src/pages/cluster/containers.rs`:

```rust
use leptos::prelude::*;

#[component]
pub fn Containers() -> impl IntoView {
    view! {
        <div>
            <h1 class="text-xl font-semibold text-text-primary">"Containers"</h1>
            <p class="text-text-muted mt-2 text-sm">"LXC container management — coming soon"</p>
        </div>
    }
}
```

Create `crates/daimon-app/src/pages/cluster/storage.rs`:

```rust
use leptos::prelude::*;

#[component]
pub fn Storage() -> impl IntoView {
    view! {
        <div>
            <h1 class="text-xl font-semibold text-text-primary">"Storage"</h1>
            <p class="text-text-muted mt-2 text-sm">"Storage management — coming soon"</p>
        </div>
    }
}
```

Create `crates/daimon-app/src/pages/incident_detail.rs`:

```rust
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

#[component]
pub fn IncidentDetail() -> impl IntoView {
    let params = use_params_map();
    let id = move || params.get().get("id").unwrap_or_default();

    view! {
        <div>
            <h1 class="text-xl font-semibold text-text-primary">"Incident "{id}</h1>
            <p class="text-text-muted mt-2 text-sm">"Incident detail view — coming soon"</p>
        </div>
    }
}
```

Create `crates/daimon-app/src/pages/settings.rs`:

```rust
use leptos::prelude::*;

#[component]
pub fn Settings() -> impl IntoView {
    view! {
        <div>
            <h1 class="text-xl font-semibold text-text-primary">"Settings"</h1>
            <p class="text-text-muted mt-2 text-sm">"Application settings — coming soon"</p>
        </div>
    }
}
```

Create `crates/daimon-app/src/pages/login.rs` (basic placeholder, auth wired in Chunk 4):

```rust
use leptos::prelude::*;

#[component]
pub fn Login() -> impl IntoView {
    view! {
        <div class="min-h-screen flex items-center justify-center bg-surface-primary">
            <div class="w-full max-w-sm p-8 bg-surface-secondary rounded-lg border border-border-primary">
                <h1 class="text-xl font-bold text-center mb-6">
                    <span class="text-text-primary">"dai"</span>
                    <span class="text-accent-amber">"mon"</span>
                </h1>
                <form class="space-y-4">
                    <div>
                        <label class="block text-sm text-text-secondary mb-1">"Username"</label>
                        <input
                            type="text"
                            class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                            placeholder="admin"
                        />
                    </div>
                    <div>
                        <label class="block text-sm text-text-secondary mb-1">"Password"</label>
                        <input
                            type="password"
                            class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                        />
                    </div>
                    <button
                        type="submit"
                        class="w-full py-2 bg-accent-amber text-surface-primary font-medium rounded-md hover:bg-accent-amber/90 transition-colors text-sm"
                    >
                        "Sign in"
                    </button>
                </form>
            </div>
        </div>
    }
}
```

- [ ] **Step 2: Add pages module to lib.rs**

Update `crates/daimon-app/src/lib.rs`:

```rust
pub mod app;
pub mod components;
pub mod pages;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use app::*;
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
}
```

- [ ] **Step 3: Commit pages**

```bash
git add -A
git commit -m "Add placeholder pages — login, dashboard, cluster, incidents, settings"
```

### Task 6: Wire up router with all routes

**Files:**
- Modify: `crates/daimon-app/src/app.rs`

- [ ] **Step 1: Replace app.rs with full routing**

Replace `crates/daimon-app/src/app.rs` with:

```rust
use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{ParentRoute, Route, Router, Routes},
    ParamSegment, StaticSegment,
};

use crate::components::layout::Layout;
use crate::pages::{
    login::Login,
    dashboard::Dashboard,
    incidents::Incidents,
    incident_detail::IncidentDetail,
    cluster::{nodes::Nodes, vms::Vms, containers::Containers, storage::Storage},
    settings::Settings,
};

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone() />
                <HydrationScripts options/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/daimon.css"/>
        <Title text="daimon"/>
        <Router>
            <Routes fallback=|| "Page not found.".into_view()>
                // Login (no layout wrapper)
                <Route path=StaticSegment("login") view=Login />

                // All other routes wrapped in Layout (sidebar + main)
                <ParentRoute path=StaticSegment("") view=Layout>
                    <Route path=StaticSegment("") view=Dashboard />
                    <Route path=StaticSegment("incidents") view=Incidents />
                    <Route path=(StaticSegment("incidents"), ParamSegment("id")) view=IncidentDetail />
                    <Route path=(StaticSegment("cluster"), StaticSegment("nodes")) view=Nodes />
                    <Route path=(StaticSegment("cluster"), StaticSegment("vms")) view=Vms />
                    <Route path=(StaticSegment("cluster"), StaticSegment("containers")) view=Containers />
                    <Route path=(StaticSegment("cluster"), StaticSegment("storage")) view=Storage />
                    <Route path=StaticSegment("settings") view=Settings />
                </ParentRoute>
            </Routes>
        </Router>
    }
}
```

- [ ] **Step 2: Verify all routes work**

```bash
cargo leptos watch
```

Test in browser:
- `http://127.0.0.1:3000/login` — login form, no sidebar
- `http://127.0.0.1:3000/` — dashboard with sidebar
- `http://127.0.0.1:3000/cluster/nodes` — nodes page with sidebar
- `http://127.0.0.1:3000/settings` — settings page
- Click sidebar links — navigation works, active state highlights

- [ ] **Step 3: Commit**

```bash
git add crates/daimon-app/src/app.rs
git commit -m "Wire up router — all pages accessible via sidebar navigation"
```

---

## Chunk 4: Auth (SQLite + JWT + Login)

### Task 7: Create SQLite database module

**Files:**
- Create: `crates/daimon-app/src/db.rs`

- [ ] **Step 1: Write db.rs test — user creation and lookup**

Add to `crates/daimon-app/src/db.rs`:

```rust
#[cfg(feature = "ssr")]
use rusqlite::{Connection, params};

#[cfg(feature = "ssr")]
pub fn init_db(path: &str) -> Connection {
    let conn = Connection::open(path).expect("Failed to open database");
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'admin',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            expires_at TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (user_id) REFERENCES users(id)
        );
        CREATE TABLE IF NOT EXISTS config (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
    ").expect("Failed to create tables");
    conn
}

#[cfg(feature = "ssr")]
pub fn find_user(conn: &Connection, username: &str) -> Option<(i64, String, String)> {
    conn.query_row(
        "SELECT id, username, password_hash FROM users WHERE username = ?1",
        params![username],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).ok()
}

#[cfg(feature = "ssr")]
pub fn create_user(conn: &Connection, username: &str, password_hash: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO users (username, password_hash) VALUES (?1, ?2)",
        params![username, password_hash],
    )?;
    Ok(conn.last_insert_rowid())
}

#[cfg(feature = "ssr")]
pub fn insert_session(conn: &Connection, id: &str, user_id: i64, expires_at: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO sessions (id, user_id, expires_at) VALUES (?1, ?2, ?3)",
        params![id, user_id, expires_at],
    )?;
    Ok(())
}

#[cfg(feature = "ssr")]
pub fn find_valid_session(conn: &Connection, id: &str) -> Option<(String, i64, String)> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().to_string();
    conn.query_row(
        "SELECT id, user_id, expires_at FROM sessions WHERE id = ?1 AND expires_at > ?2",
        params![id, now],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).ok()
}

#[cfg(feature = "ssr")]
pub fn delete_session(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
    Ok(())
}

#[cfg(feature = "ssr")]
pub fn get_config(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM config WHERE key = ?1",
        params![key],
        |row| row.get(0),
    ).ok()
}

#[cfg(feature = "ssr")]
pub fn set_config(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::*;

    #[test]
    fn test_user_crud() {
        let conn = init_db(":memory:");
        let id = create_user(&conn, "admin", "$2b$12$hash").unwrap();
        assert!(id > 0);

        let (uid, username, hash) = find_user(&conn, "admin").unwrap();
        assert_eq!(uid, id);
        assert_eq!(username, "admin");
        assert_eq!(hash, "$2b$12$hash");

        assert!(find_user(&conn, "nonexistent").is_none());
    }

    #[test]
    fn test_session_crud() {
        let conn = init_db(":memory:");
        let user_id = create_user(&conn, "admin", "hash").unwrap();

        insert_session(&conn, "sess-123", user_id, "2026-12-31T23:59:59Z").unwrap();

        let (sid, uid, exp) = find_session(&conn, "sess-123").unwrap();
        assert_eq!(sid, "sess-123");
        assert_eq!(uid, user_id);
        assert_eq!(exp, "2026-12-31T23:59:59Z");

        delete_session(&conn, "sess-123").unwrap();
        assert!(find_session(&conn, "sess-123").is_none());
    }

    #[test]
    fn test_config_crud() {
        let conn = init_db(":memory:");
        set_config(&conn, "jwt_secret", "mysecret").unwrap();
        assert_eq!(get_config(&conn, "jwt_secret").unwrap(), "mysecret");

        set_config(&conn, "jwt_secret", "newsecret").unwrap();
        assert_eq!(get_config(&conn, "jwt_secret").unwrap(), "newsecret");
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cd crates/daimon-app && cargo test --features ssr
```

Expected: 3 tests pass.

- [ ] **Step 3: Add db module to lib.rs**

Add `pub mod db;` to `crates/daimon-app/src/lib.rs`.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "Add SQLite database module — users, sessions, config tables with tests"
```

### Task 8: Create auth module (JWT + bcrypt)

**Files:**
- Create: `crates/daimon-app/src/auth.rs`

- [ ] **Step 1: Write auth.rs with tests**

Create `crates/daimon-app/src/auth.rs`:

```rust
#[cfg(feature = "ssr")]
use jsonwebtoken::{encode, decode, Header, Algorithm, Validation, EncodingKey, DecodingKey};
#[cfg(feature = "ssr")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,       // username
    pub user_id: i64,
    pub exp: usize,        // expiry (unix timestamp)
    pub session_id: String,
}

#[cfg(feature = "ssr")]
pub fn hash_password(password: &str) -> String {
    bcrypt::hash(password, 12).expect("Failed to hash password")
}

#[cfg(feature = "ssr")]
pub fn verify_password(password: &str, hash: &str) -> bool {
    bcrypt::verify(password, hash).unwrap_or(false)
}

#[cfg(feature = "ssr")]
pub fn create_jwt(secret: &str, username: &str, user_id: i64, session_id: &str) -> String {
    let expiry = chrono_free_expiry(); // 24h from now
    let claims = Claims {
        sub: username.to_string(),
        user_id,
        exp: expiry,
        session_id: session_id.to_string(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    ).expect("Failed to create JWT")
}

#[cfg(feature = "ssr")]
pub fn validate_jwt(secret: &str, token: &str) -> Option<Claims> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    )
    .ok()
    .map(|data| data.claims)
}

#[cfg(feature = "ssr")]
fn chrono_free_expiry() -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    (now + 86400) as usize // 24 hours
}

#[cfg(feature = "ssr")]
pub fn generate_secret() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::rng().random();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::*;

    #[test]
    fn test_password_hash_and_verify() {
        let hash = hash_password("mypassword");
        assert!(verify_password("mypassword", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn test_jwt_roundtrip() {
        let secret = "test-secret-key";
        let token = create_jwt(secret, "admin", 1, "sess-abc");
        let claims = validate_jwt(secret, &token).unwrap();
        assert_eq!(claims.sub, "admin");
        assert_eq!(claims.user_id, 1);
        assert_eq!(claims.session_id, "sess-abc");
    }

    #[test]
    fn test_jwt_invalid_secret() {
        let token = create_jwt("secret1", "admin", 1, "sess-abc");
        assert!(validate_jwt("secret2", &token).is_none());
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cd crates/daimon-app && cargo test --features ssr
```

Expected: 6 tests pass (3 db + 3 auth).

- [ ] **Step 3: Add auth module to lib.rs**

Add `pub mod auth;` to `crates/daimon-app/src/lib.rs`.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "Add auth module — JWT creation/validation, bcrypt password hashing with tests"
```

### Task 9: Create shared AppState and wire auth into server + login page

**Files:**
- Create: `crates/daimon-app/src/state.rs` (AppState — shared between main.rs and server functions)
- Modify: `crates/daimon-app/src/main.rs` (init DB, seed admin, provide state via context)
- Modify: `crates/daimon-app/src/pages/login.rs` (server function for login)
- Modify: `crates/daimon-app/src/lib.rs` (add state module)

- [ ] **Step 1: Create state.rs (shared AppState)**

Create `crates/daimon-app/src/state.rs`:

```rust
#[cfg(feature = "ssr")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub jwt_secret: String,
}
```

Add `pub mod state;` to `crates/daimon-app/src/lib.rs`.

- [ ] **Step 2: Update main.rs with DB init and Axum state**

Replace `crates/daimon-app/src/main.rs`:

```rust
#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::Router;
    use leptos::logging::log;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use daimon_app::app::*;
    use daimon_app::db;
    use daimon_app::auth;
    use std::sync::{Arc, Mutex};

    // Init database
    let conn = db::init_db("daimon.db");

    // Ensure JWT secret exists
    let jwt_secret = match db::get_config(&conn, "jwt_secret") {
        Some(secret) => secret,
        None => {
            let secret = auth::generate_secret();
            db::set_config(&conn, "jwt_secret", &secret).unwrap();
            secret
        }
    };

    // Seed admin user if no users exist
    if db::find_user(&conn, "admin").is_none() {
        let password = std::env::var("DAIMON_ADMIN_PASSWORD")
            .unwrap_or_else(|_| {
                let pwd = auth::generate_secret();
                let short = &pwd[..16.min(pwd.len())];
                log!("Generated admin password: {}", short);
                short.to_string()
            });
        let hash = auth::hash_password(&password);
        db::create_user(&conn, "admin", &hash).unwrap();
        log!("Admin user created");
    }

    let conf = get_configuration(None).unwrap();
    let addr = conf.leptos_options.site_addr;
    let leptos_options = conf.leptos_options;
    let routes = generate_route_list(App);

    // Share DB connection and JWT secret via Leptos context
    let db = std::sync::Arc::new(std::sync::Mutex::new(conn));
    let app_state = daimon_app::state::AppState {
        db: db.clone(),
        jwt_secret: jwt_secret.clone(),
    };

    let app = Router::new()
        .leptos_routes_with_context(
            &leptos_options,
            routes,
            {
                let app_state = app_state.clone();
                move || {
                    leptos::context::provide_context(app_state.clone());
                }
            },
            {
                let leptos_options = leptos_options.clone();
                move || shell(leptos_options.clone())
            },
        )
        .fallback(leptos_axum::file_and_error_handler(shell))
        .with_state(leptos_options);

    log!("daimon listening on http://{}", &addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

#[cfg(not(feature = "ssr"))]
pub fn main() {}
```

- [ ] **Step 2: Wire login form to server function**

Replace `crates/daimon-app/src/pages/login.rs`:

```rust
use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

#[cfg(feature = "ssr")]
use {
    crate::auth,
    crate::db,
    crate::state::AppState,
    axum::http::header::SET_COOKIE,
};

#[server]
async fn login_action(username: String, password: String) -> Result<bool, ServerFnError> {
    let state = expect_context::<AppState>();
    let conn = state.db.lock().unwrap();

    let (user_id, _username, hash) = db::find_user(&conn, &username)
        .ok_or_else(|| ServerFnError::new("Invalid credentials"))?;

    if !auth::verify_password(&password, &hash) {
        return Err(ServerFnError::new("Invalid credentials"));
    }

    let session_id = auth::generate_secret();
    let expiry_secs = 86400; // 24h
    let expiry_ts = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        now + expiry_secs
    };
    let expires_at = format!("{}", expiry_ts);

    db::insert_session(&conn, &session_id, user_id, &expires_at).unwrap();

    let token = auth::create_jwt(&state.jwt_secret, &username, user_id, &session_id);

    // Set cookie via response headers
    let cookie = format!(
        "daimon_token={}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}",
        token, expiry_secs
    );
    let response_options = expect_context::<leptos_axum::ResponseOptions>();
    response_options.insert_header(SET_COOKIE, cookie.parse().unwrap());

    Ok(true)
}

#[component]
pub fn Login() -> impl IntoView {
    let login = ServerAction::<LoginAction>::new();
    let navigate = use_navigate();
    let (error, set_error) = signal(Option::<String>::None);

    // Redirect on success
    Effect::new(move || {
        if let Some(Ok(true)) = login.value().get() {
            navigate("/", Default::default());
        }
        if let Some(Err(e)) = login.value().get() {
            set_error.set(Some(e.to_string()));
        }
    });

    view! {
        <div class="min-h-screen flex items-center justify-center bg-surface-primary">
            <div class="w-full max-w-sm p-8 bg-surface-secondary rounded-lg border border-border-primary">
                <h1 class="text-xl font-bold text-center mb-6">
                    <span class="text-text-primary">"dai"</span>
                    <span class="text-accent-amber">"mon"</span>
                </h1>

                <Show when=move || error.get().is_some()>
                    <div class="mb-4 p-2 bg-accent-danger/10 border border-accent-danger/30 rounded text-accent-danger text-sm text-center">
                        {move || error.get().unwrap_or_default()}
                    </div>
                </Show>

                <ActionForm action=login attr:class="space-y-4">
                    <div>
                        <label class="block text-sm text-text-secondary mb-1">"Username"</label>
                        <input
                            type="text"
                            name="username"
                            class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                            placeholder="admin"
                            required
                        />
                    </div>
                    <div>
                        <label class="block text-sm text-text-secondary mb-1">"Password"</label>
                        <input
                            type="password"
                            name="password"
                            class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                            required
                        />
                    </div>
                    <button
                        type="submit"
                        class="w-full py-2 bg-accent-amber text-surface-primary font-medium rounded-md hover:bg-accent-amber/90 transition-colors text-sm"
                        disabled=move || login.pending().get()
                    >
                        {move || if login.pending().get() { "Signing in..." } else { "Sign in" }}
                    </button>
                </ActionForm>
            </div>
        </div>
    }
}
```

Note: The Leptos 0.8 server function API may need minor adjustments during implementation. The `expect_context` pattern is the canonical way to access shared state in Leptos 0.8 server functions.

- [ ] **Step 3: Verify login flow**

```bash
DAIMON_ADMIN_PASSWORD=test123 cargo leptos watch
```

- Go to `http://127.0.0.1:3000/login`
- Enter admin / test123
- Should redirect to dashboard
- Check browser cookies — `daimon_token` should be set

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "Wire auth — SQLite init, admin seed, JWT login flow, cookie session"
```

### Task 9.5: Add route protection (redirect unauthenticated users)

**Files:**
- Create: `crates/daimon-app/src/auth_guard.rs`
- Modify: `crates/daimon-app/src/components/layout.rs` (check auth on mount)
- Modify: `crates/daimon-app/src/lib.rs` (add auth_guard module)

- [ ] **Step 1: Create get_current_user server function**

Create `crates/daimon-app/src/auth_guard.rs`:

```rust
use leptos::prelude::*;

#[server]
pub async fn get_current_user() -> Result<Option<String>, ServerFnError> {
    use crate::state::AppState;
    use crate::auth;
    use crate::db;

    let state = expect_context::<AppState>();

    // Read cookie from request
    let req_parts = expect_context::<leptos_axum::RequestParts>();
    let cookie = req_parts
        .headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Extract daimon_token from cookie string
    let token = cookie
        .split(';')
        .find_map(|c| {
            let c = c.trim();
            c.strip_prefix("daimon_token=")
        });

    let Some(token) = token else {
        return Ok(None);
    };

    // Validate JWT
    let Some(claims) = auth::validate_jwt(&state.jwt_secret, token) else {
        return Ok(None);
    };

    // Validate session exists and not expired
    let conn = state.db.lock().unwrap();
    if db::find_valid_session(&conn, &claims.session_id).is_none() {
        return Ok(None);
    }

    Ok(Some(claims.sub))
}
```

- [ ] **Step 2: Update Layout to check auth and redirect**

Replace `crates/daimon-app/src/components/layout.rs`:

```rust
use leptos::prelude::*;
use leptos_router::components::Outlet;
use leptos_router::hooks::use_navigate;
use super::sidebar::Sidebar;
use crate::auth_guard::get_current_user;

#[component]
pub fn Layout() -> impl IntoView {
    let user = Resource::new(|| (), |_| get_current_user());
    let navigate = use_navigate();

    // Redirect to login if not authenticated
    Effect::new(move || {
        if let Some(Ok(None)) = user.get() {
            navigate("/login", Default::default());
        }
    });

    view! {
        <Suspense fallback=|| view! { <div class="min-h-screen bg-surface-primary" /> }>
            {move || user.get().map(|result| match result {
                Ok(Some(_username)) => view! {
                    <div class="flex min-h-screen bg-surface-primary text-text-primary">
                        <Sidebar />
                        <main class="flex-1 min-w-0 p-4 sm:p-6">
                            <div class="max-w-[1400px] mx-auto">
                                <Outlet />
                            </div>
                        </main>
                    </div>
                }.into_any(),
                _ => view! { <div class="min-h-screen bg-surface-primary" /> }.into_any(),
            })}
        </Suspense>
    }
}
```

- [ ] **Step 3: Add module to lib.rs**

Add `pub mod auth_guard;` to `crates/daimon-app/src/lib.rs`.

- [ ] **Step 4: Verify route protection**

```bash
DAIMON_ADMIN_PASSWORD=test123 cargo leptos watch
```

- Open `http://127.0.0.1:3000/` without logging in — should redirect to `/login`
- Login with admin/test123 — should land on dashboard
- Open `http://127.0.0.1:3000/cluster/nodes` — should work (authenticated)
- Clear cookies — should redirect to `/login`

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Add route protection — redirect unauthenticated users to login"
```

### Task 10: Push to remotes

**Files:** None

- [ ] **Step 1: Push all Phase 4 work**

```bash
git push
```

Verify both GitHub and GitLab receive the commits. Check GitHub Actions CI passes.

---

## Implementation Notes

### Leptos 0.8 API caveats

The code in this plan is based on the official `start-axum` template for Leptos 0.8.17. During implementation:

1. Use `expect_context::<AppState>()` in server functions (not `extract()`)
2. Use `leptos_routes_with_context` in main.rs to provide AppState to server functions
3. If `ParentRoute` doesn't exist in 0.8, use nested `<Route>` with layout wrapper
4. If `ServerAction` / `ActionForm` APIs changed, check Leptos 0.8 book examples
5. Run `cargo leptos watch` frequently — the compiler is your best guide
6. Add `cargo check --features ssr` between chunks for faster feedback without browser

### Tailwind 4 version

cargo-leptos auto-downloads the Tailwind CLI. Set `LEPTOS_TAILWIND_VERSION=v4.1.5` if a specific version is needed. If Tailwind download fails, install manually and set `LEPTOS_TAILWIND_BIN` env var.

### daimon-pve crate

The existing `daimon-pve` crate is untouched in this phase. It stays in the workspace but `daimon-app` does not depend on it yet. No compilation issues — cargo-leptos only builds the crate specified in `[[workspace.metadata.leptos]]`.

### Product identity

daimon is an AI-driven system engineer for Proxmox — not a monitoring platform. Monitoring is built into every page as a capability the AI uses. There is no separate "Monitoring" page. The AI layer (floating chatbot + agentic incident pipeline) is added in future phases.
