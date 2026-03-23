# daimon Phase 4: Skeleton UI

## Overview

Full Rust stack skeleton using Leptos (SSR + WASM hydration) and Axum. Ports DataGate's enterprise layout as the UI foundation, adapted for Proxmox-native navigation. Single binary with embedded assets.

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
│   │   ├── src/
│   │   │   ├── lib.rs            (shared: components, pages, routes)
│   │   │   ├── main.rs           (server entrypoint: Axum)
│   │   │   ├── auth.rs           (JWT + bcrypt)
│   │   │   ├── db.rs             (SQLite via rusqlite)
│   │   │   ├── pages/
│   │   │   │   ├── login.rs
│   │   │   │   ├── dashboard.rs
│   │   │   │   ├── cluster/      (nodes, vms, containers, storage)
│   │   │   │   ├── incidents.rs
│   │   │   │   ├── monitoring.rs (placeholder)
│   │   │   │   ├── ai_console.rs (placeholder)
│   │   │   │   └── settings.rs
│   │   │   └── components/
│   │   │       ├── layout.rs     (sidebar + main content area)
│   │   │       ├── sidebar.rs    (collapsible, mobile responsive)
│   │   │       └── icons.rs      (SVG icon helper)
│   │   └── style/
│   │       └── main.css          (Tailwind entry point)
│   ├── daimon-pve/               (Proxmox API client lib — existing)
│   ├── daimon-agent/             (future: guest metrics agent)
│   └── daimon-mobile/            (future: mobile companion)
```

## Skeleton Scope

### Included

- **Login page**: username + password form, JWT session cookie, bcrypt password hash
- **Layout shell**: sidebar (collapsible, mobile responsive) + main content area
- **Sidebar navigation**:
  - Overview (dashboard)
  - Incidents
  - Cluster > Nodes / VMs / Containers / Storage
  - Monitoring (greyed out placeholder)
  - AI Console (greyed out placeholder)
  - Settings
  - Connection status indicator + user/logout
- **Empty pages**: each route renders page title + placeholder content
- **Theme**: DataGate dark + amber (#080C14 background, CSS variables via Tailwind)
- **SQLite**: auto-creates DB on first run, seeds admin user
- **Auth flow**: login -> JWT cookie -> protected routes -> logout

### Excluded (future phases)

- PVE API integration
- Real data on any page
- Monitoring / embedded TSDB
- daimon agent
- AI features / chat
- TOTP/MFA
- Mobile companion

## Tech Stack

| Layer | Choice |
|---|---|
| Frontend | Leptos 0.7 (SSR + hydration) |
| Server | Axum (via leptos_axum) |
| Build | cargo-leptos |
| CSS | Tailwind 4 |
| App DB | SQLite (rusqlite) |
| Auth | JWT (jsonwebtoken) + bcrypt (bcrypt crate) |
| Binary | Single binary, embedded assets |

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
Monitoring        [placeholder]
AI Console        [placeholder]
Settings
─────────
Connection status
User / Logout
```

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
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (username) REFERENCES users(username)
);
```

Admin user auto-seeded on first run with configurable password (env var or generated).

## Auth Flow

1. User submits login form (Leptos server function)
2. Server verifies bcrypt hash against SQLite
3. Server creates JWT, sets HttpOnly cookie
4. Client-side routing checks auth state via server function
5. Protected routes redirect to /login if no valid session
6. Logout clears cookie + removes session from DB

## Design Decisions

- **Leptos SSR over CSR**: faster first paint, SEO irrelevant but SSR enables server functions (direct DB access without REST API layer)
- **SQLite over PostgreSQL**: zero-config deployment, single file, battery-included philosophy
- **cargo-leptos**: standard build tool, contributors will expect it
- **DataGate theme as-is**: proven design, rebrand later when product has shape
- **Proxmox-native nav**: Nodes/VMs/Containers/Storage mirrors PVE's own UI mental model
- **No REST API layer for skeleton**: Leptos server functions call SQLite directly. REST API added later when agent needs it.
