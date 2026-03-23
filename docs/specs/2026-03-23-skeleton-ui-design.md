# daimon Phase 4: Skeleton UI

## Overview

Full Rust stack skeleton for daimon — an AI-driven system engineer for Proxmox. Uses Leptos (SSR + WASM hydration) and Axum. Ports DataGate's enterprise layout as the UI foundation, adapted for Proxmox-native navigation. Monitoring is built into every page (not a separate section) because daimon's monitoring serves the AI layer. Server binary + site/ assets directory (embeddable via rust-embed in future release builds).

## Architecture

```
Browser (WASM) <-> Axum Server <-> SQLite (app state)
                       |
                   PVE API (future phases)
```

Build tool: cargo-leptos (handles WASM compilation, CSS bundling, hot reload).

## Crate Structure

```
daimon/
├── Cargo.toml                    (workspace)
├── crates/
│   ├── daimon-app/               (Leptos full-stack: Axum server + WASM client)
│   │   ├── Cargo.toml
│   │   ├── Leptos.toml           (cargo-leptos config: output name, site root, style, etc.)
│   │   ├── src/
│   │   │   ├── app.rs            (root App component: Router + Routes)
│   │   │   ├── lib.rs            (re-exports: app, components, pages)
│   │   │   ├── main.rs           (server entrypoint: Axum — only compiled with "ssr" feature)
│   │   │   ├── auth.rs           (JWT + bcrypt — server-only)
│   │   │   ├── db.rs             (SQLite via rusqlite — server-only)
│   │   │   ├── state.rs          (shared AppState — accessible by server functions)
│   │   │   ├── pages/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── login.rs
│   │   │   │   ├── dashboard.rs
│   │   │   │   ├── cluster/      (mod.rs, nodes.rs, vms.rs, containers.rs, storage.rs)
│   │   │   │   ├── incidents.rs
│   │   │   │   ├── incident_detail.rs
│   │   │   │   └── settings.rs
│   │   │   └── components/
│   │   │       ├── mod.rs
│   │   │       ├── layout.rs     (sidebar + main content area)
│   │   │       ├── sidebar.rs    (collapsible, mobile responsive)
│   │   │       └── icons.rs      (SVG icon helper)
│   │   └── style/
│   │       └── main.css          (Tailwind entry point)
│   ├── daimon-pve/               (Proxmox API client lib — existing, not depended on this phase)
│   ├── daimon-agent/             (future: guest metrics agent)
│   └── daimon-mobile/            (future: mobile companion)
```

**daimon-bin disposition**: Replaced by daimon-app. Remove crates/daimon-bin/ during Phase 4 implementation. The binary output name remains `daimon`.

## Leptos Feature Flags

daimon-app uses Leptos' standard dual-compilation model:

```toml
[features]
ssr = ["leptos/ssr", "leptos_axum", "leptos_router/ssr", "rusqlite", "jsonwebtoken", "bcrypt"]
hydrate = ["leptos/hydrate", "leptos_router/hydrate"]
```

- **ssr**: Server-side rendering. Compiles main.rs, auth.rs, db.rs. Includes all server-only deps.
- **hydrate**: Client-side hydration. Compiles to WASM. No server deps.
- cargo-leptos automatically builds with `--features ssr` for the server binary and `--features hydrate` for WASM.
- Server-only code (auth, db, API calls) guarded with `#[cfg(feature = "ssr")]`.

## Leptos.toml

```toml
[leptos]
output-name = "daimon"
site-root = "target/site"
site-pkg-dir = "pkg"
style-file = "style/main.css"
bin-features = ["ssr"]
lib-features = ["hydrate"]
```

## Tailwind 4 Integration

Tailwind 4 standalone CLI generates CSS from `style/main.css`. cargo-leptos watches the style file and triggers rebuild. The CSS entry point uses:

```css
@import "tailwindcss";
```

Tailwind scans `.rs` files for class names (configured via `@source` directive pointing at `src/`).

## Skeleton Scope

### Included

- **Login page**: username + password form, JWT session cookie, bcrypt password hash
- **Layout shell**: sidebar (collapsible, mobile responsive) + main content area
- **Sidebar navigation**:
  - Overview (dashboard)
  - Incidents
  - Cluster > Nodes / VMs / Containers / Storage
  - Settings
  - Connection status indicator + user/logout
  - Note: No separate "Monitoring" or "AI Console" menu items. Monitoring is built into each page. AI is a floating chatbot (future phase).
- **Empty pages**: each route renders page title + placeholder content
- **Theme**: DataGate dark + amber (#080C14 background, CSS variables via Tailwind)
- **SQLite**: auto-creates DB on first run, seeds admin user
- **Auth flow**: login -> JWT cookie -> protected routes -> logout

### Excluded (future phases)

- PVE API integration (daimon-pve not depended on this phase)
- Real data on any page
- Monitoring / embedded TSDB
- daimon agent
- AI features / chat
- TOTP/MFA
- Mobile companion

## Tech Stack

| Layer | Choice |
|---|---|
| Frontend | Leptos 0.8 (SSR + hydration) |
| Server | Axum (via leptos_axum) |
| Build | cargo-leptos |
| CSS | Tailwind 4 (standalone CLI) |
| App DB | SQLite (rusqlite) |
| Auth | JWT (jsonwebtoken) + bcrypt |
| Binary | Server binary + site/ directory |

## Theme (ported from DataGate)

- Background: #080C14
- Surface primary/secondary/tertiary via CSS variables
- Accent: amber
- Text: light gray hierarchy (primary/secondary/muted)
- Dark mode only (no light mode toggle)

## Sidebar Navigation Structure

```
Overview
Incidents
Cluster
  ├── Nodes
  ├── VMs
  ├── Containers
  └── Storage
Settings
─────────
Connection status
User / Logout
```

No "Monitoring" menu — monitoring is built into each page (Nodes shows node metrics, VMs shows VM metrics, etc.). No "AI Console" menu — AI is a floating chatbot available everywhere (future phase).

Ported from DataGate's collapsible sidebar with:
- Chevron expand/collapse for sections
- Active route highlighting (amber accent)
- Mobile hamburger menu + overlay
- Badge support (for future incident counts)

## Database Schema (minimal for skeleton)

```sql
CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'admin',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (user_id) REFERENCES users(id)
);
```

All timestamps stored as UTC in the database. Display-layer converts to MYT (UTC+8).

Admin user auto-seeded on first run. Password from `DAIMON_ADMIN_PASSWORD` env var, or auto-generated and printed to stdout on first boot.

## Auth Flow

1. User submits login form (Leptos server function)
2. Server verifies bcrypt hash against SQLite
3. Server creates JWT (HS256), inserts session row, sets cookie
4. Cookie attributes: `HttpOnly`, `SameSite=Lax`, `Path=/`, `Secure` in production
5. Protected routes: server function checks JWT signature + validates session exists in DB (supports revocation)
6. JWT TTL: 24 hours. Session row tracks expiry for server-side validation.
7. Logout: clears cookie + deletes session row from DB
8. JWT signing secret: generated on first boot, persisted to SQLite config table. Survives restarts, sessions stay valid.

## Route Table

```
/login                          Login (public)
/                               Dashboard (Overview)
/incidents                      Incidents list
/incidents/:id                  Incident detail
/cluster/nodes                  Nodes
/cluster/vms                    Virtual Machines
/cluster/containers             Containers
/cluster/storage                Storage
/settings                       Settings
```

All routes except /login are protected (redirect to /login if no valid session).

## Design Decisions

- **Leptos SSR over CSR**: faster first paint, SSR enables server functions (direct DB access without REST API layer)
- **SQLite over PostgreSQL**: zero-config deployment, single file, battery-included philosophy
- **cargo-leptos**: standard build tool, contributors will expect it
- **DataGate theme as-is**: proven design, rebrand later when product has shape
- **Proxmox-native nav**: Nodes/VMs/Containers/Storage mirrors PVE's own UI mental model
- **No REST API layer for skeleton**: Leptos server functions call SQLite directly. REST API added later when agent needs it
- **Server binary + site/ dir**: cargo-leptos default output. Single-binary embedding (rust-embed) deferred to release packaging phase
- **Session DB check on every request**: JWT is verified by signature AND checked against sessions table. Slightly slower but enables logout/revocation without token blacklists
- **Rust edition 2024**: matches existing workspace config. Requires rustc 1.85+. Leptos 0.8 is compatible.
- **No separate Monitoring page**: daimon is an AI-driven system engineer, not a monitoring platform. Monitoring is a capability built into each page — Nodes shows node health, VMs shows VM metrics, etc. The AI layer consumes monitoring data to investigate and act.
- **No separate AI Console page**: AI is a floating chatbot bubble (like DataGate's ChatBubble) available from any page. Added in a future phase.
