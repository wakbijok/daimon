# Phase 5.5: Rich Tables, Detail Views & Real-Time Data

## Overview

Upgrade dAImon's data presentation layer from basic static tables to a fully-featured monitoring interface with sortable/searchable/paginated tables, entity detail views with RRD sparklines, real-time WebSocket updates, theme switching, and data export. Built as a reusable component library that all future phases will use.

Design principle: **Build the component library first, then wire into pages.** Every component is reusable across nodes, VMs, LXCs, and storage — and ready for Phase 6+ agent data.

## Architecture: Component Library

Five new reusable components plus one refactored module:

```
components/
├── mod.rs              (existing — add re-exports)
├── sortable_table.rs   (NEW — generic sorted/filtered/searchable/paginated table)
├── sparkline.rs        (NEW — SVG mini-chart, inline or standalone)
├── auto_refresh.rs     (NEW — WebSocket with polling fallback, interval selector)
├── detail_layout.rs    (NEW — tabbed detail page skeleton with agent-prompt)
├── summary_bar.rs      (NEW — reusable stat cards for detail view headers)
├── theme.rs            (NEW — dark/light theme signal + toggle + persistence)
├── table.rs            (REFACTOR — NodeTable/GuestTable/StorageTable become thin wrappers)
└── ... (existing unchanged)
```

### SortableTable

Generic table component parameterized over row type `T`.

**TableRow trait:**
```rust
trait TableRow: Clone + 'static {
    fn columns() -> Vec<ColumnDef>;
    fn cell_value(&self, col: &str) -> String;       // for sort/filter (string comparison)
    fn cell_view(&self, col: &str) -> impl IntoView;  // for rendering (rich HTML)
    fn row_key(&self) -> String;                       // unique key for reconciliation
    fn row_link(&self) -> Option<String>;              // click-through URL (None = no navigation)
}

struct ColumnDef {
    key: String,
    label: String,
    sortable: bool,
    default_hidden: bool,    // for column toggle
    sort_type: SortType,     // Text, Numeric, Percentage
}
```

**Built-in features (all client-side signals, no server round-trips):**
- Click-to-sort: unsorted → ascending → descending → unsorted cycle. Arrow indicator in header. One column at a time.
- Text search: single input above table, debounced 150ms, filters across all visible columns.
- Pagination: configurable page size (25/50/100), page navigation controls. Default: 25.
- Column visibility toggle: dropdown button in table header. Hidden columns excluded from search. Persisted per-table via `user_preferences` DB table.
- Row click: navigates to `row_link()` URL. Cursor pointer on hover.
- Export: CSV and JSON buttons. Exports current filtered/sorted view (not just current page).

### Sparkline

SVG-based mini-chart component.

**Props:**
- `data: Vec<f64>` — data points
- `color: String` — stroke color (theme-aware)
- `width: u32, height: u32` — dimensions
- `fill: bool` — gradient fill under line (default true)
- `show_current: bool` — display current value text beside chart

**Rendering:** Pure SVG polyline with optional linear gradient fill. No JS charting library dependency. Points normalized to viewBox. Handles empty data (shows flat line or "No data" text).

**Usage:** Inline in table cells (small: 80x20) and in detail view chart panels (large: 100%x80).

### AutoRefresh / WebSocket

Real-time data layer with graceful degradation.

**Dependencies:**
- Server: `axum` with `ws` feature enabled (Axum 0.8 includes WebSocket support via `axum::extract::ws`)
- Client (WASM): `web-sys` with additional features: `WebSocket`, `MessageEvent`, `CloseEvent`, `ErrorEvent`
- No external WebSocket crate needed — Axum's built-in WS + browser native WebSocket API

**Primary: WebSocket**
- Server exposes `ws://host/api/v1/ws` endpoint
- Client subscribes to metric streams per entity (node, VM, LXC, storage)
- Server pushes updates as metrics change (from PVE API polling server-side)
- JSON text messages (simple, debuggable; switch to MessagePack later if bandwidth becomes an issue)

**WebSocket message protocol:**

```rust
// Client → Server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum WsClientMsg {
    Subscribe { scope: WsScope },      // subscribe to entity updates
    Unsubscribe { scope: WsScope },    // unsubscribe
    Ping,                               // keepalive
}

// Server → Client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum WsServerMsg {
    Snapshot { scope: WsScope, data: serde_json::Value },  // full state on subscribe
    Update { scope: WsScope, data: serde_json::Value },    // incremental update
    Pong,                                                    // keepalive response
    Error { message: String },                               // subscription error
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WsScope {
    ClusterResources { cluster_id: String },                         // all resources in a cluster
    NodeRrd { cluster_id: String, node: String },                    // node RRD sparkline data
    GuestRrd { cluster_id: String, node: String, vmid: u64 },       // VM/LXC RRD data
    StorageRrd { cluster_id: String, node: String, storage: String },// storage RRD data
}
```

**Subscription handshake:**
1. Client opens WebSocket to `/api/v1/ws`
2. Client sends `Subscribe { scope: ClusterResources { cluster_id } }`
3. Server replies with `Snapshot` (full current state)
4. Server pushes `Update` messages when data changes
5. Client sends `Unsubscribe` when navigating away
6. Keepalive: client sends `Ping` every 30s, server responds with `Pong`. Server closes connection after 60s without `Ping`.

**Axum integration:**
- WebSocket route registered BEFORE the Leptos fallback route in `main.rs`: `Router::route("/api/v1/ws", get(ws_handler))`
- `ws_handler` uses `axum::extract::ws::WebSocketUpgrade` extractor
- `AppState` passed via `axum::Extension(state)` (separate from Leptos state injection)

**Fallback: HTTP polling**
- If WebSocket connection fails or disconnects, falls back to polling at configured interval
- Automatic reconnection with exponential backoff (1s, 2s, 4s, 8s, max 30s)
- Polling uses existing Leptos `Resource` + server function pattern (no code change needed)

**Interval selector:**
- Dropdown in top-right of table area: 5s / 15s / 30s / 60s / Off
- Default: 30s
- Circular countdown indicator shows time until next refresh
- Applies globally (stored as signal, persisted in user_preferences)
- "Off" pauses all auto-refresh (useful when inspecting data)
- Controls the server-side PVE polling interval AND the client polling fallback interval

**Server-side PVE cache + polling:**

```rust
struct PveCache {
    /// Latest cluster resources per cluster, keyed by cluster_id
    resources: HashMap<String, Vec<PveResource>>,
    /// Latest RRD data per (cluster_id, node), keyed by composite key
    node_rrd: HashMap<(String, String), Vec<RrdDataPoint>>,
    /// Timestamp of last poll per cluster
    last_poll: HashMap<String, Instant>,
}
```

- Background `tokio::spawn` task runs a polling loop:
  1. Sleeps for configured interval (default 30s)
  2. For each registered cluster: calls `cluster_resources()` + `node_rrddata()` for online nodes
  3. Compares new data against cached data — if changed, stores new snapshot
  4. Broadcasts `Update` to all WebSocket subscribers for affected scopes
- "Changed" detection: compare serialized JSON of old vs new (simple, correct; optimize later if needed)
- Poll interval updated dynamically when user changes interval selector (sent via a `tokio::watch` channel)
- Multiple browser clients share one PVE polling loop — N clients = 1 PVE API call, not N

### DetailLayout

Reusable tabbed detail page skeleton.

**Props:**
- `title: String` — entity name (e.g., "nargothrond")
- `subtitle: Option<String>` — secondary info (e.g., API URL)
- `tabs: Vec<TabDef>` — tab configuration
- `children: Children` — outlet for tab content

**TabDef:**
```rust
struct TabDef {
    label: String,
    route: String,
    requires_agent: bool,
    agent_description: String,  // shown in placeholder
}
```

**Agent placeholder:** When `requires_agent` is true and no agent is connected, the tab content area shows:
- Info icon
- "Install daimon-agent for [label] data"
- Brief description of what data the agent provides
- Install command: `curl -fsSL https://daimon.dev/install.sh | sh`

### SummaryBar

Horizontal stat cards for detail view headers.

**Props:** `items: Vec<SummaryItem>`

```rust
struct SummaryItem {
    label: String,          // "CPU", "Memory", "Status"
    value: String,          // "12.4%", "78.2%", "Online"
    color: Option<String>,  // value color (green for online, amber for CPU, etc.)
    sparkline: Option<Vec<f64>>,  // optional inline sparkline beside value
}
```

Renders as a horizontal flex row of cards with label on top, value below, optional sparkline.

### Theme

Dark/light theme support.

**Implementation:**
- CSS variables defined in `tailwind.css` for both themes (already have dark, add light variants)
- `ThemeSignal` stored in Leptos context — reactive, toggles class on `<html>` element
- Tailwind `dark:` variant for component-level overrides where needed
- Toggle button in user menu (sun/moon icon)
- Persisted in `user_preferences` (key: `theme`, value: `dark`/`light`)
- Default: `dark` (matches current design)
- System preference detection: if no saved preference, follow `prefers-color-scheme`

**Light theme colors:**
```css
/* Light mode overrides */
--color-surface-primary: #FFFFFF
--color-surface-secondary: #F6F8FA
--color-surface-tertiary: #F0F2F5
--color-text-primary: #1F2328
--color-text-secondary: #656D76
--color-text-muted: #8B949E
--color-border-primary: #D0D7DE
/* Accent colors stay the same (amber, green, danger, purple) */
```

## Data Layer: PVE API Extensions

### New Client Methods (daimon-pve crate)

| Method | PVE Endpoint | Returns |
|--------|-------------|---------|
| `node_rrddata(node, timeframe)` | `/nodes/{node}/rrddata` | `Vec<RrdDataPoint>` |
| `qemu_rrddata(node, vmid, timeframe)` | `/nodes/{node}/qemu/{vmid}/rrddata` | `Vec<RrdDataPoint>` |
| `lxc_rrddata(node, vmid, timeframe)` | `/nodes/{node}/lxc/{vmid}/rrddata` | `Vec<RrdDataPoint>` |
| `storage_rrddata(node, storage, timeframe)` | `/nodes/{node}/storage/{storage}/rrddata` | `Vec<RrdDataPoint>` |
| `qemu_status(node, vmid)` | `/nodes/{node}/qemu/{vmid}/status/current` | `QemuStatus` |
| `lxc_status(node, vmid)` | `/nodes/{node}/lxc/{vmid}/status/current` | `LxcStatus` |
| `qemu_config(node, vmid)` | `/nodes/{node}/qemu/{vmid}/config` | `GuestConfig` |
| `lxc_config(node, vmid)` | `/nodes/{node}/lxc/{vmid}/config` | `GuestConfig` |

### New Types

**Note on field types:** PVE RRD endpoints return all numeric values as `f64` (JSON floats), while the existing `PveResource` type uses `u64` for `mem`, `maxmem`, `disk`, `maxdisk`. These are intentionally different types — `RrdDataPoint` models RRD time-series data, `PveResource` models current resource snapshots. Do not attempt to unify them.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RrdDataPoint {
    time: f64,
    cpu: Option<f64>,
    maxcpu: Option<f64>,
    mem: Option<f64>,
    maxmem: Option<f64>,
    disk: Option<f64>,
    maxdisk: Option<f64>,
    netin: Option<f64>,
    netout: Option<f64>,
    diskread: Option<f64>,
    diskwrite: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum RrdTimeframe {
    Hour,    // ~70 points, 1min resolution
    Day,     // ~70 points, 20min resolution
    Week,    // ~70 points, 3h resolution
    Month,   // ~70 points, 12h resolution
    Year,    // ~70 points, 1week resolution
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QemuStatus {
    pid: Option<u64>,
    qmpstatus: Option<String>,
    running_machine: Option<String>,
    running_qemu: Option<String>,
    ha: Option<HaStatus>,
    // Flattened metrics (same fields as PveResource, duplicated here for self-contained detail view)
    status: String,
    cpu: Option<f64>,
    maxcpu: Option<u64>,
    mem: Option<u64>,
    maxmem: Option<u64>,
    disk: Option<u64>,
    maxdisk: Option<u64>,
    netin: Option<u64>,
    netout: Option<u64>,
    uptime: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LxcStatus {
    pid: Option<u64>,
    ha: Option<HaStatus>,
    // Flattened metrics (same as QemuStatus)
    status: String,
    cpu: Option<f64>,
    maxcpu: Option<u64>,
    mem: Option<u64>,
    maxmem: Option<u64>,
    disk: Option<u64>,
    maxdisk: Option<u64>,
    netin: Option<u64>,
    netout: Option<u64>,
    uptime: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GuestConfig {
    cores: Option<u32>,
    memory: Option<u64>,       // MB
    balloon: Option<u64>,      // MB, if ballooning enabled
    sockets: Option<u32>,
    net: Vec<String>,          // network device configs
    scsi: Vec<String>,         // disk configs
    ide: Vec<String>,
    virtio: Vec<String>,
}
```

### Data Flow

**Tables (list views):**
1. Page loads → fetch `cluster_resources` (current data, as today)
2. In parallel → fetch `node_rrddata(node, Hour)` for each online node (sparkline data)
3. SortableTable renders immediately with current data; sparklines fill in asynchronously
4. WebSocket pushes metric updates; table rows update reactively
5. If WS disconnects, falls back to HTTP polling at configured interval

**Detail views:**
1. Row click → navigate to detail route
2. Detail page fetches: status + config + RRD data (hour for overview)
3. Charts tab allows timeframe selection (hour/day/week/month/year)
4. Agent-required tabs show placeholder until agent is connected
5. WebSocket provides real-time updates to overview stats and charts

## Detail View Pages

### Node Detail

**Route:** `/clusters/:cluster_id/nodes/:node_name`

**Summary bar:** Status, Uptime, CPU% (with sparkline), Memory% (with sparkline), PVE version, Kernel version

**Tabs:**

| Tab | Content | Data Source | Phase |
|-----|---------|-------------|-------|
| Overview | 4 chart panels (CPU, Memory, Network I/O, Disk I/O) + guest list on this node | PVE RRD + cluster/resources | 5.5 |
| Hardware | IPMI sensors table (temp, fan, voltage, PSU) | Agent (host mode) | 6 |
| RAID | Controller info, virtual drives, physical drives, cache policy, BBU | Agent (host mode) | 6 |
| Disks | Per-disk SMART, temperature, utilization, slot mapping | Agent (host mode) | 6 |
| Storage | Local mounts, NFS/CIFS/iSCSI/FC, multipath | Agent (host mode) | 6 |
| Network | Per-NIC traffic, errors, bond/bridge status | Agent (host mode) | 6 |
| Services | PVE daemon states (pvedaemon, pveproxy, corosync, ceph) | Agent (host mode) | 6 |
| Charts | Full-size RRD charts with timeframe selector (hour/day/week/month/year) | PVE RRD | 5.5 |

**Guest list on Overview:** Grid of compact cards showing VMID, name, type (VM/LXC), status indicator. Clickable → navigates to VM/LXC detail.

### VM Detail

**Route:** `/clusters/:cluster_id/vms/:vmid`

**Summary bar:** Status, Node, VMID, Uptime, CPU% (with sparkline), Memory (allocated vs used), Disk (allocated vs used)

**Tabs:**

| Tab | Content | Data Source | Phase |
|-----|---------|-------------|-------|
| Overview | Config summary (cores, RAM, disks, NICs) + 4 chart panels | PVE API + RRD | 5.5 |
| Processes | Top by CPU/RAM, zombie count, process list | Agent (guest mode) | 6 |
| Services | systemd units, Docker containers | Agent (guest mode) | 6 |
| Network | Listening ports, connections, per-interface traffic | Agent (guest mode) | 6 |
| Logs | Recent journal entries, filterable by unit/priority | Agent (guest mode) | 6 |
| Charts | Full-size RRD charts with timeframe selector | PVE RRD | 5.5 |

### LXC Detail

**Route:** `/clusters/:cluster_id/containers/:vmid`

Same structure as VM Detail. Same tabs, same layout. Different data fetch (LXC API endpoints).

### Storage Detail

**Route:** `/clusters/:cluster_id/nodes/:node_name/storage/:storage_name`

**Note:** Storage names are not globally unique within a PVE cluster (e.g., `local` exists on every node). The route includes `:node_name` to disambiguate. The PVE `storage_rrddata` endpoint also requires a `node` parameter. The storage list table shows the node column so users know which node's storage they're clicking into.

**Summary bar:** Type, Node, Content types, Total/Used/Available, Status (active/inactive)

**Tabs:**

| Tab | Content | Data Source | Phase |
|-----|---------|-------------|-------|
| Overview | Usage breakdown + chart panel | PVE API + RRD | 5.5 |
| Devices | Per-disk status, temp, SMART | Agent (host mode) | 6 |
| Usage | Which VMs/LXCs use this pool, how much each | PVE API | 5.5 |
| Charts | Full-size RRD charts with timeframe selector | PVE RRD | 5.5 |

## Routing

New routes added to existing Leptos router:

```
/clusters/:cluster_id/nodes/:node_name              → NodeDetail (parent)
  /clusters/:cluster_id/nodes/:node_name/hardware    → placeholder (agent)
  /clusters/:cluster_id/nodes/:node_name/raid        → placeholder (agent)
  /clusters/:cluster_id/nodes/:node_name/disks       → placeholder (agent)
  /clusters/:cluster_id/nodes/:node_name/storage     → placeholder (agent)
  /clusters/:cluster_id/nodes/:node_name/network     → placeholder (agent)
  /clusters/:cluster_id/nodes/:node_name/services    → placeholder (agent)
  /clusters/:cluster_id/nodes/:node_name/charts      → RRD charts

/clusters/:cluster_id/vms/:vmid                      → VmDetail (parent)
  /clusters/:cluster_id/vms/:vmid/processes           → placeholder (agent)
  /clusters/:cluster_id/vms/:vmid/services            → placeholder (agent)
  /clusters/:cluster_id/vms/:vmid/network             → placeholder (agent)
  /clusters/:cluster_id/vms/:vmid/logs                → placeholder (agent)
  /clusters/:cluster_id/vms/:vmid/charts              → RRD charts

/clusters/:cluster_id/containers/:vmid               → LxcDetail (parent)
  (same sub-routes as VM)

/clusters/:cluster_id/nodes/:node_name/storage/:storage_name          → StorageDetail (parent)
  /clusters/:cluster_id/nodes/:node_name/storage/:storage_name/devices → placeholder (agent)
  /clusters/:cluster_id/nodes/:node_name/storage/:storage_name/usage   → PVE API data
  /clusters/:cluster_id/nodes/:node_name/storage/:storage_name/charts  → RRD charts
```

Default child route for each detail view: Overview (index route).

## File Structure

```
crates/daimon-pve/src/
├── types.rs            (MODIFY — add RrdDataPoint, RrdTimeframe, QemuStatus, LxcStatus, GuestConfig)
├── client.rs           (MODIFY — add 8 new API methods)

crates/daimon-app/src/
├── app.rs              (MODIFY — add all detail view routes)
├── state.rs            (MODIFY — add PVE cache + WebSocket state)
├── ws.rs               (NEW — WebSocket endpoint + subscription manager)
├── components/
│   ├── mod.rs           (MODIFY — re-export new components)
│   ├── sortable_table.rs    (NEW — generic table with sort/search/paginate/export/column toggle)
│   ├── sparkline.rs         (NEW — SVG sparkline component)
│   ├── auto_refresh.rs      (NEW — WebSocket client + polling fallback + interval selector)
│   ├── detail_layout.rs     (NEW — tabbed detail skeleton + agent prompt)
│   ├── summary_bar.rs       (NEW — horizontal stat cards)
│   ├── theme.rs             (NEW — dark/light toggle + persistence)
│   ├── table.rs             (REFACTOR — thin wrappers using SortableTable)
│   └── icons.rs             (MODIFY — add sun/moon icons)
├── pages/
│   ├── cluster/
│   │   ├── mod.rs           (MODIFY — add detail page modules)
│   │   ├── nodes.rs         (MODIFY — wire SortableTable + sparklines + row click)
│   │   ├── vms.rs           (MODIFY — same)
│   │   ├── containers.rs    (MODIFY — same)
│   │   ├── storage.rs       (MODIFY — same)
│   │   ├── node_detail.rs       (NEW — node detail parent + overview)
│   │   ├── node_charts.rs       (NEW — full RRD charts with timeframe selector)
│   │   ├── vm_detail.rs         (NEW — VM detail parent + overview)
│   │   ├── vm_charts.rs         (NEW — VM RRD charts)
│   │   ├── lxc_detail.rs        (NEW — LXC detail parent + overview)
│   │   ├── lxc_charts.rs        (NEW — LXC RRD charts)
│   │   ├── storage_detail.rs    (NEW — storage detail parent + overview)
│   │   ├── storage_usage.rs     (NEW — which VMs/LXCs use this pool)
│   │   ├── storage_charts.rs    (NEW — storage RRD charts)
│   │   └── agent_placeholder.rs (NEW — reusable "install agent" page)

style/
├── tailwind.css        (MODIFY — add light theme variables + dark: overrides)
```

## Testing

| What | How | Estimated tests |
|------|-----|-----------------|
| RRD type deserialization | Unit test with sample PVE JSON fixtures | 4 |
| New PVE client methods | Unit test URL construction + response parsing | 5 |
| SortableTable sort logic | Unit test sort functions across SortType variants | 4 |
| SortableTable filter logic | Unit test text matching across columns | 3 |
| SortableTable pagination | Unit test page slicing + boundary conditions | 3 |
| Sparkline SVG generation | Unit test point-to-path conversion + edge cases | 2 |
| Export CSV/JSON | Unit test serialization output format | 2 |
| Theme persistence | Unit test preference read/write | 1 |
| WebSocket message protocol | Unit test WsClientMsg/WsServerMsg/WsScope serialization + deserialization | 4 |
| WebSocket subscription logic | Unit test subscribe/unsubscribe/broadcast to correct scopes | 3 |
| PveCache change detection | Unit test diff logic (changed vs unchanged data) | 2 |

**~33 new tests**, bringing total from 12 to ~45.

No integration tests against live PVE — manual homelab testing.

## Out of Scope

- VM/LXC management actions (start/stop/restart/migrate) — by design, dAImon observes; AI acts through agents
- Multi-user RBAC — single admin sufficient for now
- Mobile-responsive layout — deferred
- Custom dashboard builder — not a Grafana replacement

## Design Decisions

- **Component library first**: Reusable components (SortableTable, Sparkline, DetailLayout) built before pages. Every future phase benefits.
- **TableRow trait**: Generic table via trait implementation. Adding a new table type = implement trait, get sort/search/paginate/export free.
- **SVG sparklines**: No JS charting library dependency. Pure SVG polylines render fast, scale with container, theme-aware.
- **WebSocket primary, polling fallback**: Real-time data for AI observation engine (Phase 7+). Graceful degradation if WS unavailable.
- **Server-side PVE cache**: daimon-app polls PVE once, caches in memory, pushes to N browser clients. Avoids N clients × M PVE API calls.
- **RRD hour for table sparklines**: 1-hour window shows recent trends at a glance. Detail view charts offer all timeframes.
- **Theme via CSS variables**: Single set of variables, swapped by class on `<html>`. Tailwind `dark:` for component overrides. System preference detection as default.
- **Column toggle persisted**: Per-table visibility saved in user_preferences. Survives page reload and sessions.
- **Export current view**: CSV/JSON exports respect current filter and sort, but include all pages (not just current page). Users get what they see, minus pagination.
- **All agent tabs stubbed**: Full navigation structure in place. Placeholder messages explain what data is missing and how to install the agent. Zero friction when Phase 6 agent lands.
- **Read-only by design**: No infrastructure management actions in UI. dAImon is eyes and brain, not hands. Hands come through the agent execution pipeline (Phase 9).
- **PVE RRD now, MetricsStore later**: Phase 5.5 sparklines and charts source data from PVE's built-in RRD endpoints. When the agent lands (Phase 6+), charts will migrate to MetricsStore data — the Sparkline and chart components are data-source agnostic (they take `Vec<f64>`), so no UI changes needed.
- **Export filenames**: CSV/JSON downloads use `{entity-type}_{cluster-name}_{YYYY-MM-DD}.csv` convention (e.g., `nodes_nargothrond_2026-03-24.csv`).
- **Storage scoped by node**: Storage routes include `:node_name` because PVE storage names are not cluster-unique (`local` exists on every node). RRD endpoint also requires node parameter.
