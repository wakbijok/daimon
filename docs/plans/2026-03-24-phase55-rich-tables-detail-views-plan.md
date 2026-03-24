# Phase 5.5: Rich Tables, Detail Views & Real-Time Data — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Upgrade dAImon from basic static PVE tables to a full monitoring interface with sortable/searchable/paginated tables, entity detail views, RRD sparklines, WebSocket real-time updates, dark/light theme, and CSV/JSON export.

**Architecture:** Component library first (SortableTable, Sparkline, DetailLayout, Theme), then wire into pages. PVE API extensions provide data. WebSocket adds real-time updates as final layer. Each chunk builds on the previous and produces working, testable software independently.

**Tech Stack:** Rust, Leptos 0.8, Axum 0.8, SQLite (rusqlite), Tailwind CSS 4, SVG (sparklines), WebSocket (Axum built-in + browser native)

**Spec:** `docs/specs/2026-03-24-phase55-rich-tables-detail-views-design.md`

**Implementation notes:**
- Chunks 1-3 (Tasks 1-10) provide complete, copy-paste-ready code. These are the foundation components.
- Chunks 4-5 (Tasks 11-19) define interfaces, types, and patterns but leave component wiring at a higher level since they follow patterns established in Chunks 1-3. Agentic executors should reference the component library code from earlier tasks.
- `vmid` uses `u32` throughout (matching PVE API and existing codebase), not `u64` as in the spec. PVE VMIDs are always within u32 range.
- Detail view routes are OUTSIDE the `ClusterDetail` ParentRoute to avoid double tab bars. Navigation from cluster tab → detail view replaces the cluster view, with a back link.
- Search filtering uses 150ms debounce via `set_timeout` (not immediate on every keystroke).
- Column visibility is persisted to `user_preferences` DB via server function on change.
- Export filenames follow spec convention: `{entity-type}_{cluster-name}_{YYYY-MM-DD}.csv`
- Row click uses `use_navigate()` for SPA navigation (not full page reload).

---

## Chunk 1: PVE API Extensions (Data Layer)

New types and client methods in the `daimon-pve` crate. Everything else depends on this.

### Task 1: Add RRD and status types to daimon-pve

**Files:**
- Modify: `crates/daimon-pve/src/types.rs`
- Modify: `crates/daimon-pve/src/lib.rs` (already re-exports `types::*`, no change needed)

- [ ] **Step 1: Add serde Serialize derive to existing types that will be sent over WebSocket**

In `crates/daimon-pve/src/types.rs`, add `Serialize` to `PveResource` derive (line 12). It currently only has `Deserialize`. Import `Serialize`:

```rust
use serde::{Deserialize, Serialize};
```

Change `PveResource` derive to:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
```

- [ ] **Step 2: Add RrdDataPoint and RrdTimeframe types**

Append to `crates/daimon-pve/src/types.rs`:

```rust
/// RRD time-series data point — returned by /nodes/{node}/rrddata
/// Note: All numeric values are f64 (PVE RRD returns floats), unlike PveResource which uses u64 for mem/disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RrdDataPoint {
    pub time: f64,
    #[serde(default)]
    pub cpu: Option<f64>,
    #[serde(default)]
    pub maxcpu: Option<f64>,
    #[serde(default)]
    pub mem: Option<f64>,
    #[serde(default)]
    pub maxmem: Option<f64>,
    #[serde(default)]
    pub disk: Option<f64>,
    #[serde(default)]
    pub maxdisk: Option<f64>,
    #[serde(default)]
    pub netin: Option<f64>,
    #[serde(default)]
    pub netout: Option<f64>,
    #[serde(default)]
    pub diskread: Option<f64>,
    #[serde(default)]
    pub diskwrite: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum RrdTimeframe {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

impl RrdTimeframe {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }
}
```

- [ ] **Step 3: Add QemuStatus, LxcStatus, GuestConfig, HaStatus types**

Append to `crates/daimon-pve/src/types.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaStatus {
    #[serde(default)]
    pub managed: Option<u8>,
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QemuStatus {
    #[serde(default)]
    pub pid: Option<u64>,
    #[serde(default)]
    pub qmpstatus: Option<String>,
    #[serde(default, rename = "running-machine")]
    pub running_machine: Option<String>,
    #[serde(default, rename = "running-qemu")]
    pub running_qemu: Option<String>,
    #[serde(default)]
    pub ha: Option<HaStatus>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub cpu: Option<f64>,
    #[serde(default)]
    pub maxcpu: Option<u64>,
    #[serde(default)]
    pub mem: Option<u64>,
    #[serde(default)]
    pub maxmem: Option<u64>,
    #[serde(default)]
    pub disk: Option<u64>,
    #[serde(default)]
    pub maxdisk: Option<u64>,
    #[serde(default)]
    pub netin: Option<u64>,
    #[serde(default)]
    pub netout: Option<u64>,
    #[serde(default)]
    pub uptime: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LxcStatus {
    #[serde(default)]
    pub pid: Option<u64>,
    #[serde(default)]
    pub ha: Option<HaStatus>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub cpu: Option<f64>,
    #[serde(default)]
    pub maxcpu: Option<u64>,
    #[serde(default)]
    pub mem: Option<u64>,
    #[serde(default)]
    pub maxmem: Option<u64>,
    #[serde(default)]
    pub disk: Option<u64>,
    #[serde(default)]
    pub maxdisk: Option<u64>,
    #[serde(default)]
    pub netin: Option<u64>,
    #[serde(default)]
    pub netout: Option<u64>,
    #[serde(default)]
    pub uptime: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
}

/// VM/LXC configuration — PVE returns numbered keys (net0, scsi0, etc.) as flat JSON.
/// We collect them into Vecs during deserialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuestConfig {
    #[serde(default)]
    pub cores: Option<u32>,
    #[serde(default)]
    pub memory: Option<u64>,
    #[serde(default)]
    pub balloon: Option<u64>,
    #[serde(default)]
    pub sockets: Option<u32>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub ostype: Option<String>,
    // Numbered device keys collected by custom logic in the client method
    #[serde(skip)]
    pub net_devices: Vec<String>,
    #[serde(skip)]
    pub disk_devices: Vec<String>,
}
```

- [ ] **Step 4: Write tests for RRD type deserialization**

Add to bottom of `crates/daimon-pve/src/types.rs` (or in a `#[cfg(test)]` module in the same file — follow existing pattern from `client.rs`). Actually, add a new test section at the bottom of `types.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrd_data_point_deserializes_from_pve_json() {
        let json = r#"{"time":1711296000,"cpu":0.0523,"maxcpu":8,"mem":12345678,"maxmem":33554432}"#;
        let point: RrdDataPoint = serde_json::from_str(json).unwrap();
        assert_eq!(point.time, 1711296000.0);
        assert!((point.cpu.unwrap() - 0.0523).abs() < f64::EPSILON);
        assert_eq!(point.maxcpu.unwrap(), 8.0);
    }

    #[test]
    fn rrd_data_point_handles_missing_fields() {
        let json = r#"{"time":1711296000}"#;
        let point: RrdDataPoint = serde_json::from_str(json).unwrap();
        assert!(point.cpu.is_none());
        assert!(point.mem.is_none());
    }

    #[test]
    fn rrd_timeframe_as_str() {
        assert_eq!(RrdTimeframe::Hour.as_str(), "hour");
        assert_eq!(RrdTimeframe::Year.as_str(), "year");
    }

    #[test]
    fn qemu_status_deserializes_with_ha() {
        let json = r#"{"status":"running","cpu":0.05,"maxcpu":4,"mem":1073741824,"maxmem":4294967296,"uptime":86400,"ha":{"managed":1,"state":"started"},"pid":12345}"#;
        let status: QemuStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status.status, "running");
        assert_eq!(status.pid, Some(12345));
        assert!(status.ha.is_some());
        assert_eq!(status.ha.unwrap().state, Some("started".to_string()));
    }
}
```

- [ ] **Step 5: Run tests to verify**

Run: `cargo test -p daimon-pve`
Expected: All existing tests + 4 new tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/daimon-pve/src/types.rs
git commit -m "Add RRD, status, and config types to daimon-pve"
```

### Task 2: Add RRD client methods

**Files:**
- Modify: `crates/daimon-pve/src/client.rs`

- [ ] **Step 1: Add rrddata methods**

Add after the `storage()` method (line 132) in `crates/daimon-pve/src/client.rs`:

```rust
    /// GET /api2/json/nodes/{node}/rrddata — node historical metrics
    pub async fn node_rrddata(&self, node: &str, timeframe: crate::RrdTimeframe) -> Result<Vec<crate::RrdDataPoint>, Error> {
        let url = format!("{}/api2/json/nodes/{}/rrddata?timeframe={}", self.base_url, node, timeframe.as_str());
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<Vec<crate::RrdDataPoint>> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/qemu/{vmid}/rrddata — VM historical metrics
    pub async fn qemu_rrddata(&self, node: &str, vmid: u32, timeframe: crate::RrdTimeframe) -> Result<Vec<crate::RrdDataPoint>, Error> {
        let url = format!("{}/api2/json/nodes/{}/qemu/{}/rrddata?timeframe={}", self.base_url, node, vmid, timeframe.as_str());
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<Vec<crate::RrdDataPoint>> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/lxc/{vmid}/rrddata — LXC historical metrics
    pub async fn lxc_rrddata(&self, node: &str, vmid: u32, timeframe: crate::RrdTimeframe) -> Result<Vec<crate::RrdDataPoint>, Error> {
        let url = format!("{}/api2/json/nodes/{}/lxc/{}/rrddata?timeframe={}", self.base_url, node, vmid, timeframe.as_str());
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<Vec<crate::RrdDataPoint>> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/storage/{storage}/rrddata — storage historical metrics
    pub async fn storage_rrddata(&self, node: &str, storage: &str, timeframe: crate::RrdTimeframe) -> Result<Vec<crate::RrdDataPoint>, Error> {
        let url = format!("{}/api2/json/nodes/{}/storage/{}/rrddata?timeframe={}", self.base_url, node, storage, timeframe.as_str());
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<Vec<crate::RrdDataPoint>> = resp.json().await?;
        Ok(body.data)
    }
```

- [ ] **Step 2: Add status and config methods**

Add after the rrddata methods:

```rust
    /// GET /api2/json/nodes/{node}/qemu/{vmid}/status/current — detailed VM status
    pub async fn qemu_status(&self, node: &str, vmid: u32) -> Result<crate::QemuStatus, Error> {
        let url = format!("{}/api2/json/nodes/{}/qemu/{}/status/current", self.base_url, node, vmid);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<crate::QemuStatus> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/lxc/{vmid}/status/current — detailed LXC status
    pub async fn lxc_status(&self, node: &str, vmid: u32) -> Result<crate::LxcStatus, Error> {
        let url = format!("{}/api2/json/nodes/{}/lxc/{}/status/current", self.base_url, node, vmid);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: ApiResponse<crate::LxcStatus> = resp.json().await?;
        Ok(body.data)
    }

    /// GET /api2/json/nodes/{node}/qemu/{vmid}/config — VM configuration
    /// Note: PVE returns numbered keys (net0, scsi0, etc.) as flat JSON fields.
    /// We parse the raw JSON and collect device keys into Vec<String>.
    pub async fn qemu_config(&self, node: &str, vmid: u32) -> Result<crate::GuestConfig, Error> {
        let url = format!("{}/api2/json/nodes/{}/qemu/{}/config", self.base_url, node, vmid);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let text = resp.text().await?;
        let raw: ApiResponse<serde_json::Value> = serde_json::from_str(&text)?;
        let mut config: crate::GuestConfig = serde_json::from_value(raw.data.clone())?;
        // Collect numbered device keys
        if let Some(obj) = raw.data.as_object() {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    if k.starts_with("net") && k[3..].parse::<u32>().is_ok() {
                        config.net_devices.push(s.to_string());
                    } else if k.starts_with("scsi") && k[4..].parse::<u32>().is_ok()
                        || k.starts_with("ide") && k[3..].parse::<u32>().is_ok()
                        || k.starts_with("virtio") && k[6..].parse::<u32>().is_ok()
                        || k.starts_with("sata") && k[4..].parse::<u32>().is_ok()
                    {
                        config.disk_devices.push(format!("{}: {}", k, s));
                    }
                }
            }
        }
        Ok(config)
    }

    /// GET /api2/json/nodes/{node}/lxc/{vmid}/config — LXC configuration
    pub async fn lxc_config(&self, node: &str, vmid: u32) -> Result<crate::GuestConfig, Error> {
        let url = format!("{}/api2/json/nodes/{}/lxc/{}/config", self.base_url, node, vmid);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let text = resp.text().await?;
        let raw: ApiResponse<serde_json::Value> = serde_json::from_str(&text)?;
        let mut config: crate::GuestConfig = serde_json::from_value(raw.data.clone())?;
        if let Some(obj) = raw.data.as_object() {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    if k.starts_with("net") && k[3..].parse::<u32>().is_ok() {
                        config.net_devices.push(s.to_string());
                    } else if k.starts_with("rootfs") || (k.starts_with("mp") && k[2..].parse::<u32>().is_ok()) {
                        config.disk_devices.push(format!("{}: {}", k, s));
                    }
                }
            }
        }
        Ok(config)
    }
```

- [ ] **Step 3: Add URL construction tests**

Add to the existing `#[cfg(test)] mod tests` in `client.rs`:

```rust
    #[test]
    fn rrddata_url_uses_timeframe_string() {
        // Verify RrdTimeframe serializes correctly for URL params
        assert_eq!(crate::RrdTimeframe::Hour.as_str(), "hour");
        assert_eq!(crate::RrdTimeframe::Day.as_str(), "day");
        assert_eq!(crate::RrdTimeframe::Week.as_str(), "week");
        assert_eq!(crate::RrdTimeframe::Month.as_str(), "month");
        assert_eq!(crate::RrdTimeframe::Year.as_str(), "year");
    }
```

- [ ] **Step 4: Add serde_json to daimon-pve dependencies**

`serde_json` is already a dependency (line 12 of `crates/daimon-pve/Cargo.toml`). No change needed.

- [ ] **Step 5: Run tests**

Run: `cargo test -p daimon-pve`
Expected: All tests pass (existing 3 + new 5 = 8 total)

- [ ] **Step 6: Commit**

```bash
git add crates/daimon-pve/src/client.rs
git commit -m "Add RRD, status, and config API methods to PVE client"
```

### Task 3: Update daimon-app dependencies

**Files:**
- Modify: `crates/daimon-app/Cargo.toml`

- [ ] **Step 1: Add serde_json dependency (needed for WebSocket + export)**

Add to `[dependencies]` section:
```toml
serde_json = "1"
gloo-timers = { version = "0.3", optional = true }
```

Add `"dep:gloo-timers"` to the `hydrate` feature list (needed for search debounce).

- [ ] **Step 2: Add web-sys WebSocket features for hydrate**

Update existing `web-sys` line (currently line 17):
```toml
web-sys = { version = "0.3", features = ["Window", "Location", "Document", "HtmlDocument", "HtmlElement", "WebSocket", "MessageEvent", "CloseEvent", "ErrorEvent", "BinaryType", "Storage", "MediaQueryList"], optional = true }
```

- [ ] **Step 3: Enable axum ws feature**

Change axum line (currently line 22) to enable WebSocket:
```toml
axum = { version = "0.8", features = ["ws"], optional = true }
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p daimon-app --features ssr`
Run: `cargo check -p daimon-app --features hydrate`
Expected: Both compile cleanly

- [ ] **Step 5: Commit**

```bash
git add crates/daimon-app/Cargo.toml
git commit -m "Update daimon-app dependencies for WebSocket and export"
```

---

## Chunk 2: Component Library

Reusable UI components that all pages will use. Built and tested independently before wiring into existing pages.

### Task 4: Theme component + light theme CSS

**Files:**
- Create: `crates/daimon-app/src/components/theme.rs`
- Modify: `crates/daimon-app/src/components/mod.rs`
- Modify: `crates/daimon-app/style/tailwind.css`
- Modify: `crates/daimon-app/src/components/user_menu.rs` (add theme toggle)

- [ ] **Step 1: Add light theme CSS variables**

In `crates/daimon-app/style/tailwind.css`, add a second `@theme` block for light mode. The existing dark theme colors are set as defaults. Add a `.light` class override using CSS custom properties at root level. The approach: keep existing `@theme` block as-is (it defines Tailwind theme tokens), add a `.light` class that overrides the CSS custom property values.

After the existing `@theme { ... }` block, add:

```css
/* Light theme overrides — applied when <html class="light"> */
.light {
    --color-surface-primary: #FFFFFF;
    --color-surface-secondary: #F6F8FA;
    --color-surface-tertiary: #F0F2F5;
    --color-text-primary: #1F2328;
    --color-text-secondary: #656D76;
    --color-text-muted: #8B949E;
    --color-border-primary: #D0D7DE;
}
```

- [ ] **Step 2: Create theme.rs component**

Create `crates/daimon-app/src/components/theme.rs`:

```rust
use leptos::prelude::*;

/// Reactive theme signal — "dark" or "light"
#[derive(Clone, Copy)]
pub struct ThemeSignal(pub RwSignal<String>);

/// Initialize theme from user preference or system default.
/// Call once in the root Layout component.
pub fn provide_theme() {
    let theme = RwSignal::new("dark".to_string());
    provide_context(ThemeSignal(theme));

    // On hydrate: read saved preference or detect system preference
    #[cfg(feature = "hydrate")]
    {
        use wasm_bindgen::JsCast;
        if let Some(window) = web_sys::window() {
            // Check localStorage for saved preference
            if let Ok(Some(storage)) = window.local_storage() {
                if let Ok(Some(saved)) = storage.get_item("daimon_theme") {
                    theme.set(saved.clone());
                    apply_theme_class(&saved);
                    return;
                }
            }
            // Fall back to system preference
            if let Ok(mq) = window.match_media("(prefers-color-scheme: light)") {
                if let Some(mq) = mq {
                    if mq.matches() {
                        theme.set("light".to_string());
                        apply_theme_class("light");
                        return;
                    }
                }
            }
        }
        apply_theme_class("dark");
    }
}

/// Toggle between dark and light, persist to localStorage
pub fn toggle_theme() {
    if let Some(ThemeSignal(theme)) = use_context::<ThemeSignal>() {
        let new_theme = if theme.get_untracked() == "dark" { "light" } else { "dark" };
        theme.set(new_theme.to_string());

        #[cfg(feature = "hydrate")]
        {
            apply_theme_class(new_theme);
            if let Some(window) = web_sys::window() {
                if let Ok(Some(storage)) = window.local_storage() {
                    let _ = storage.set_item("daimon_theme", new_theme);
                }
            }
        }
    }
}

#[cfg(feature = "hydrate")]
fn apply_theme_class(theme: &str) {
    if let Some(window) = web_sys::window() {
        if let Some(doc) = window.document() {
            if let Some(el) = doc.document_element() {
                let _ = el.class_list().remove_1("dark");
                let _ = el.class_list().remove_1("light");
                let _ = el.class_list().add_1(theme);
            }
        }
    }
}
```

- [ ] **Step 3: Register theme module and add to layout**

In `crates/daimon-app/src/components/mod.rs`, add:
```rust
pub mod theme;
```

In `crates/daimon-app/src/components/layout.rs`, call `provide_theme()` at the top of the `Layout` component body, before the auth check.

- [ ] **Step 4: Add theme toggle button to user menu**

In `crates/daimon-app/src/components/user_menu.rs`, add a theme toggle button (sun/moon icon) that calls `toggle_theme()`.

- [ ] **Step 5: Verify compilation**

Run: `cargo leptos build`
Expected: Compiles. Dark theme still works. Toggle switches to light theme.

- [ ] **Step 6: Commit**

```bash
git add crates/daimon-app/src/components/theme.rs crates/daimon-app/src/components/mod.rs crates/daimon-app/style/tailwind.css crates/daimon-app/src/components/layout.rs crates/daimon-app/src/components/user_menu.rs
git commit -m "Add dark/light theme toggle with localStorage persistence"
```

### Task 5: Sparkline SVG component

**Files:**
- Create: `crates/daimon-app/src/components/sparkline.rs`
- Modify: `crates/daimon-app/src/components/mod.rs`

- [ ] **Step 1: Write sparkline SVG generation logic (pure function, testable)**

Create `crates/daimon-app/src/components/sparkline.rs`:

```rust
use leptos::prelude::*;

/// Convert data points to SVG polyline points string.
/// Normalizes values to fit within the given width x height viewBox.
pub fn points_to_polyline(data: &[f64], width: u32, height: u32) -> String {
    if data.is_empty() {
        return String::new();
    }
    let min = data.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = data.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = if (max - min).abs() < f64::EPSILON { 1.0 } else { max - min };
    let padding = 1.0; // 1px padding top/bottom

    data.iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = if data.len() == 1 {
                width as f64 / 2.0
            } else {
                (i as f64 / (data.len() - 1) as f64) * width as f64
            };
            let y = height as f64 - padding - ((v - min) / range) * (height as f64 - 2.0 * padding);
            format!("{:.1},{:.1}", x, y)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Inline SVG sparkline component.
#[component]
pub fn Sparkline(
    /// Data points to render
    data: Vec<f64>,
    /// Stroke color (CSS color string)
    #[prop(default = "#F59E0B".to_string())]
    color: String,
    /// Width in pixels
    #[prop(default = 80)]
    width: u32,
    /// Height in pixels
    #[prop(default = 20)]
    height: u32,
    /// Show gradient fill under line
    #[prop(default = true)]
    fill: bool,
) -> impl IntoView {
    if data.is_empty() {
        return view! { <span class="text-text-muted text-[10px]">"—"</span> }.into_any();
    }

    let points = points_to_polyline(&data, width, height);
    let gradient_id = format!("spark-{}", data.len()); // unique enough for non-reuse
    let vb = format!("0 0 {} {}", width, height);

    view! {
        <svg
            width=width.to_string()
            height=height.to_string()
            viewBox=vb
            class="inline-block align-middle"
        >
            {if fill {
                Some(view! {
                    <defs>
                        <linearGradient id=gradient_id.clone() x1="0" y1="0" x2="0" y2="1">
                            <stop offset="0%" stop-color=color.clone() stop-opacity="0.3"/>
                            <stop offset="100%" stop-color=color.clone() stop-opacity="0"/>
                        </linearGradient>
                    </defs>
                    <polyline
                        points=format!("{} {},{} 0,{}", points.clone(), width, height, height)
                        fill=format!("url(#{})", gradient_id)
                        stroke="none"
                    />
                })
            } else {
                None
            }}
            <polyline
                points=points
                fill="none"
                stroke=color
                stroke-width="1.5"
                stroke-linecap="round"
                stroke-linejoin="round"
            />
        </svg>
    }.into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_data_returns_empty_string() {
        assert_eq!(points_to_polyline(&[], 80, 20), "");
    }

    #[test]
    fn single_point_centered() {
        let result = points_to_polyline(&[5.0], 80, 20);
        assert!(result.contains("40.0")); // centered x
    }

    #[test]
    fn two_points_span_width() {
        let result = points_to_polyline(&[0.0, 10.0], 100, 20);
        let parts: Vec<&str> = result.split(' ').collect();
        assert_eq!(parts.len(), 2);
        assert!(parts[0].starts_with("0.0")); // x=0
        assert!(parts[1].starts_with("100.0")); // x=100
    }
}
```

- [ ] **Step 2: Register module**

In `crates/daimon-app/src/components/mod.rs`, add:
```rust
pub mod sparkline;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p daimon-app --lib`
Expected: Sparkline tests pass (3 new tests)

- [ ] **Step 4: Commit**

```bash
git add crates/daimon-app/src/components/sparkline.rs crates/daimon-app/src/components/mod.rs
git commit -m "Add SVG sparkline component with pure function rendering"
```

### Task 6: SortableTable trait and sort/filter/pagination logic

**Files:**
- Create: `crates/daimon-app/src/components/sortable_table.rs`
- Modify: `crates/daimon-app/src/components/mod.rs`

This is the largest component. Split into logic (pure functions, testable) and view (Leptos component).

- [ ] **Step 1: Define TableRow trait and supporting types**

Create `crates/daimon-app/src/components/sortable_table.rs` with the trait, types, and pure logic functions:

```rust
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

// --- Types ---

#[derive(Clone, Debug)]
pub struct ColumnDef {
    pub key: &'static str,
    pub label: &'static str,
    pub sortable: bool,
    pub default_hidden: bool,
    pub sort_type: SortType,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SortType {
    Text,
    Numeric,
    Percentage,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SortDir {
    Asc,
    Desc,
}

// --- Trait ---

pub trait TableRow: Clone + 'static {
    fn columns() -> Vec<ColumnDef>;
    fn cell_value(&self, col: &str) -> String;
    fn cell_view(&self, col: &str) -> AnyView;
    fn row_key(&self) -> String;
    fn row_link(&self) -> Option<String> { None }
}

// --- Pure logic functions (testable without DOM) ---

/// Sort rows by column value
pub fn sort_rows<T: TableRow>(rows: &mut [T], col: &str, dir: SortDir, sort_type: SortType) {
    rows.sort_by(|a, b| {
        let va = a.cell_value(col);
        let vb = b.cell_value(col);
        let ordering = match sort_type {
            SortType::Numeric | SortType::Percentage => {
                let na: f64 = va.parse().unwrap_or(0.0);
                let nb: f64 = vb.parse().unwrap_or(0.0);
                na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
            }
            SortType::Text => va.to_lowercase().cmp(&vb.to_lowercase()),
        };
        match dir {
            SortDir::Asc => ordering,
            SortDir::Desc => ordering.reverse(),
        }
    });
}

/// Filter rows by search query across all visible columns
pub fn filter_rows<T: TableRow>(rows: &[T], query: &str, visible_cols: &[&str]) -> Vec<T> {
    if query.is_empty() {
        return rows.to_vec();
    }
    let q = query.to_lowercase();
    rows.iter()
        .filter(|row| {
            visible_cols.iter().any(|col| {
                row.cell_value(col).to_lowercase().contains(&q)
            })
        })
        .cloned()
        .collect()
}

/// Get a page slice from rows
pub fn paginate<T: Clone>(rows: &[T], page: usize, page_size: usize) -> Vec<T> {
    let start = page * page_size;
    if start >= rows.len() {
        return vec![];
    }
    let end = (start + page_size).min(rows.len());
    rows[start..end].to_vec()
}

/// Total number of pages
pub fn total_pages(row_count: usize, page_size: usize) -> usize {
    if page_size == 0 { return 0; }
    (row_count + page_size - 1) / page_size
}

/// Export rows to CSV string
pub fn export_csv<T: TableRow>(rows: &[T], columns: &[ColumnDef]) -> String {
    let mut csv = columns.iter().map(|c| c.label).collect::<Vec<_>>().join(",");
    csv.push('\n');
    for row in rows {
        let line = columns.iter()
            .map(|c| {
                let val = row.cell_value(c.key);
                if val.contains(',') || val.contains('"') {
                    format!("\"{}\"", val.replace('"', "\"\""))
                } else {
                    val
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        csv.push_str(&line);
        csv.push('\n');
    }
    csv
}

/// Export rows to JSON string
pub fn export_json<T: TableRow>(rows: &[T], columns: &[ColumnDef]) -> String {
    let objects: Vec<serde_json::Value> = rows.iter().map(|row| {
        let mut map = serde_json::Map::new();
        for col in columns {
            map.insert(col.key.to_string(), serde_json::Value::String(row.cell_value(col.key)));
        }
        serde_json::Value::Object(map)
    }).collect();
    serde_json::to_string_pretty(&objects).unwrap_or_default()
}
```

- [ ] **Step 2: Add comprehensive tests for sort/filter/pagination/export**

Append to the same file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct TestRow {
        name: String,
        value: f64,
    }

    impl TableRow for TestRow {
        fn columns() -> Vec<ColumnDef> {
            vec![
                ColumnDef { key: "name", label: "Name", sortable: true, default_hidden: false, sort_type: SortType::Text },
                ColumnDef { key: "value", label: "Value", sortable: true, default_hidden: false, sort_type: SortType::Numeric },
            ]
        }
        fn cell_value(&self, col: &str) -> String {
            match col {
                "name" => self.name.clone(),
                "value" => format!("{:.1}", self.value),
                _ => String::new(),
            }
        }
        fn cell_view(&self, _col: &str) -> AnyView {
            ().into_any()
        }
        fn row_key(&self) -> String { self.name.clone() }
    }

    fn sample_rows() -> Vec<TestRow> {
        vec![
            TestRow { name: "charlie".into(), value: 30.0 },
            TestRow { name: "alice".into(), value: 10.0 },
            TestRow { name: "bob".into(), value: 20.0 },
        ]
    }

    #[test]
    fn sort_text_asc() {
        let mut rows = sample_rows();
        sort_rows(&mut rows, "name", SortDir::Asc, SortType::Text);
        assert_eq!(rows[0].name, "alice");
        assert_eq!(rows[2].name, "charlie");
    }

    #[test]
    fn sort_numeric_desc() {
        let mut rows = sample_rows();
        sort_rows(&mut rows, "value", SortDir::Desc, SortType::Numeric);
        assert_eq!(rows[0].value, 30.0);
        assert_eq!(rows[2].value, 10.0);
    }

    #[test]
    fn sort_text_case_insensitive() {
        let mut rows = vec![
            TestRow { name: "Banana".into(), value: 1.0 },
            TestRow { name: "apple".into(), value: 2.0 },
        ];
        sort_rows(&mut rows, "name", SortDir::Asc, SortType::Text);
        assert_eq!(rows[0].name, "apple");
    }

    #[test]
    fn filter_matches_any_column() {
        let rows = sample_rows();
        let result = filter_rows(&rows, "bob", &["name", "value"]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "bob");
    }

    #[test]
    fn filter_case_insensitive() {
        let rows = sample_rows();
        let result = filter_rows(&rows, "ALICE", &["name"]);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn filter_empty_query_returns_all() {
        let rows = sample_rows();
        let result = filter_rows(&rows, "", &["name"]);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn paginate_first_page() {
        let rows = sample_rows();
        let page = paginate(&rows, 0, 2);
        assert_eq!(page.len(), 2);
    }

    #[test]
    fn paginate_last_page_partial() {
        let rows = sample_rows();
        let page = paginate(&rows, 1, 2);
        assert_eq!(page.len(), 1); // 3 items, page_size 2, page 1 = 1 item
    }

    #[test]
    fn paginate_beyond_range() {
        let rows = sample_rows();
        let page = paginate(&rows, 5, 2);
        assert!(page.is_empty());
    }

    #[test]
    fn total_pages_calculation() {
        assert_eq!(total_pages(0, 25), 0);
        assert_eq!(total_pages(25, 25), 1);
        assert_eq!(total_pages(26, 25), 2);
        assert_eq!(total_pages(100, 25), 4);
    }

    #[test]
    fn export_csv_basic() {
        let rows = sample_rows();
        let cols = TestRow::columns();
        let csv = export_csv(&rows, &cols);
        assert!(csv.starts_with("Name,Value\n"));
        assert!(csv.contains("alice,10.0"));
    }

    #[test]
    fn export_csv_escapes_commas() {
        let rows = vec![TestRow { name: "a,b".into(), value: 1.0 }];
        let cols = TestRow::columns();
        let csv = export_csv(&rows, &cols);
        assert!(csv.contains("\"a,b\""));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p daimon-app --lib -- sortable_table`
Expected: All 12 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/daimon-app/src/components/sortable_table.rs crates/daimon-app/src/components/mod.rs
git commit -m "Add SortableTable trait with sort, filter, pagination, and export logic"
```

### Task 7: SortableTable Leptos component (view layer)

**Files:**
- Modify: `crates/daimon-app/src/components/sortable_table.rs`

- [ ] **Step 1: Add the SortableTable Leptos component**

This component uses the pure functions from Task 6. Append to `sortable_table.rs`, after the tests module:

```rust
// --- Leptos Component ---

/// Renders a full-featured table with sorting, filtering, pagination, column toggle, export, and row click.
#[component]
pub fn SortableTable<T: TableRow + Send + Sync>(
    rows: Vec<T>,
    /// Unique table ID for persisting column visibility preferences
    #[prop(default = "default")]
    table_id: &'static str,
) -> impl IntoView {
    let columns = T::columns();
    let all_rows = StoredValue::new(rows);

    // Signals for table state
    let (search_query, set_search_query) = signal(String::new());
    let (sort_col, set_sort_col) = signal::<Option<String>>(None);
    let (sort_dir, set_sort_dir) = signal(SortDir::Asc);
    let (current_page, set_current_page) = signal(0usize);
    let (page_size, set_page_size) = signal(25usize);
    let (hidden_cols, set_hidden_cols) = signal::<Vec<String>>(
        columns.iter().filter(|c| c.default_hidden).map(|c| c.key.to_string()).collect()
    );
    let (show_col_menu, set_show_col_menu) = signal(false);

    let columns_stored = StoredValue::new(columns.clone());

    // Derived: visible columns
    let visible_columns = move || {
        let hidden = hidden_cols.get();
        columns_stored.get_value().into_iter().filter(move |c| !hidden.contains(&c.key.to_string())).collect::<Vec<_>>()
    };

    // Derived: processed rows (filter → sort → full list for export)
    let processed_rows = move || {
        let cols = visible_columns();
        let visible_keys: Vec<&str> = cols.iter().map(|c| c.key).collect();
        let mut filtered = filter_rows(&all_rows.get_value(), &search_query.get(), &visible_keys);

        if let Some(ref col) = sort_col.get() {
            if let Some(def) = cols.iter().find(|c| c.key == col.as_str()) {
                sort_rows(&mut filtered, col, sort_dir.get(), def.sort_type);
            }
        }
        filtered
    };

    // Derived: current page rows
    let page_rows = move || {
        let all = processed_rows();
        paginate(&all, current_page.get(), page_size.get())
    };

    let total_filtered = move || processed_rows().len();
    let num_pages = move || total_pages(total_filtered(), page_size.get());

    // Click header to cycle sort
    let on_header_click = move |key: String| {
        let current = sort_col.get();
        if current.as_deref() == Some(&key) {
            match sort_dir.get() {
                SortDir::Asc => set_sort_dir.set(SortDir::Desc),
                SortDir::Desc => {
                    set_sort_col.set(None); // reset
                }
            }
        } else {
            set_sort_col.set(Some(key));
            set_sort_dir.set(SortDir::Asc);
        }
        set_current_page.set(0);
    };

    // Reset page on search (debounced 150ms)
    let debounce_handle = StoredValue::new(None::<leptos::task::JoinHandle<()>>);
    let on_search = move |ev: leptos::ev::Event| {
        let val = event_target_value(&ev);
        // Cancel previous debounce timer
        if let Some(handle) = debounce_handle.get_value() {
            handle.abort();
        }
        // Set new 150ms debounce
        let handle = leptos::task::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(150).await;
            set_search_query.set(val);
            set_current_page.set(0);
        });
        debounce_handle.set_value(Some(handle));
    };

    // Export handlers
    let on_export_csv = move |_| {
        let rows = processed_rows();
        let cols = visible_columns();
        let csv = export_csv(&rows, &cols);
        #[cfg(feature = "hydrate")]
        trigger_download(&csv, &format!("{}_{}.csv", table_id, "export"), "text/csv");
    };

    let on_export_json = move |_| {
        let rows = processed_rows();
        let cols = visible_columns();
        let json = export_json(&rows, &cols);
        #[cfg(feature = "hydrate")]
        trigger_download(&json, &format!("{}_{}.json", table_id, "export"), "application/json");
    };

    view! {
        <div class="space-y-3">
            // Toolbar: search + column toggle + export + page size
            <div class="flex items-center gap-2 flex-wrap">
                <input
                    type="text"
                    placeholder="Search..."
                    on:input=on_search
                    class="px-3 py-1.5 text-sm bg-surface-secondary border border-border-primary rounded-md text-text-primary placeholder-text-muted focus:outline-none focus:border-accent-amber w-64"
                />
                <div class="flex-1"></div>
                // Column toggle
                <div class="relative">
                    <button
                        on:click=move |_| set_show_col_menu.update(|v| *v = !*v)
                        class="px-2 py-1.5 text-xs text-text-muted border border-border-primary rounded-md hover:text-text-secondary"
                    >
                        "Columns"
                    </button>
                    <Show when=move || show_col_menu.get()>
                        <div class="absolute right-0 top-8 z-10 bg-surface-secondary border border-border-primary rounded-md shadow-lg p-2 min-w-40">
                            {move || columns_stored.get_value().iter().map(|col| {
                                let key = col.key.to_string();
                                let key2 = key.clone();
                                let label = col.label;
                                let is_hidden = move || hidden_cols.get().contains(&key);
                                view! {
                                    <label class="flex items-center gap-2 py-1 px-2 text-xs text-text-secondary hover:bg-surface-tertiary rounded cursor-pointer">
                                        <input
                                            type="checkbox"
                                            checked=move || !is_hidden()
                                            on:change=move |_| {
                                                set_hidden_cols.update(|v| {
                                                    if v.contains(&key2) {
                                                        v.retain(|k| k != &key2);
                                                    } else {
                                                        v.push(key2.clone());
                                                    }
                                                });
                                            }
                                            class="rounded"
                                        />
                                        {label}
                                    </label>
                                }
                            }).collect_view()}
                        </div>
                    </Show>
                </div>
                // Export buttons
                <button on:click=on_export_csv class="px-2 py-1.5 text-xs text-text-muted border border-border-primary rounded-md hover:text-text-secondary">"CSV"</button>
                <button on:click=on_export_json class="px-2 py-1.5 text-xs text-text-muted border border-border-primary rounded-md hover:text-text-secondary">"JSON"</button>
                // Page size selector
                <select
                    on:change=move |ev| {
                        let val: usize = event_target_value(&ev).parse().unwrap_or(25);
                        set_page_size.set(val);
                        set_current_page.set(0);
                    }
                    class="px-2 py-1.5 text-xs bg-surface-secondary border border-border-primary rounded-md text-text-muted"
                >
                    <option value="25" selected=true>"25"</option>
                    <option value="50">"50"</option>
                    <option value="100">"100"</option>
                </select>
            </div>

            // Table
            <table class="w-full text-sm">
                <thead>
                    <tr class="border-b border-border-primary text-text-muted text-[11px] uppercase tracking-wider">
                        {move || visible_columns().into_iter().map(|col| {
                            let key = col.key.to_string();
                            let key2 = key.clone();
                            let is_sorted = move || sort_col.get().as_deref() == Some(&key);
                            let current_dir = move || if is_sorted() { Some(sort_dir.get()) } else { None };
                            view! {
                                <th
                                    class="text-left py-3 px-4 font-medium select-none"
                                    class:cursor-pointer=col.sortable
                                    on:click=move |_| {
                                        if col.sortable {
                                            on_header_click(key2.clone());
                                        }
                                    }
                                >
                                    <span class="inline-flex items-center gap-1">
                                        {col.label}
                                        {move || match current_dir() {
                                            Some(SortDir::Asc) => " ↑",
                                            Some(SortDir::Desc) => " ↓",
                                            None => "",
                                        }}
                                    </span>
                                </th>
                            }
                        }).collect_view()}
                    </tr>
                </thead>
                <tbody>
                    {move || page_rows().into_iter().map(|row| {
                        let cols = visible_columns();
                        let link = row.row_link();
                        let has_link = link.is_some();
                        let navigate = leptos_router::hooks::use_navigate();
                        view! {
                            <tr
                                class="border-b border-border-primary/50 hover:bg-surface-tertiary/50"
                                class:cursor-pointer=has_link
                                on:click=move |_| {
                                    if let Some(ref href) = link {
                                        navigate(href, Default::default());
                                    }
                                }
                            >
                                {cols.iter().map(|col| {
                                    view! {
                                        <td class="py-3 px-4">{row.cell_view(col.key)}</td>
                                    }
                                }).collect_view()}
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>

            // Pagination footer
            <div class="flex items-center justify-between text-xs text-text-muted">
                <span>{move || format!("Showing {} of {} rows", page_rows().len(), total_filtered())}</span>
                <div class="flex items-center gap-1">
                    <button
                        on:click=move |_| if current_page.get() > 0 { set_current_page.update(|p| *p -= 1); }
                        disabled=move || current_page.get() == 0
                        class="px-2 py-1 border border-border-primary rounded disabled:opacity-30"
                    >
                        "Prev"
                    </button>
                    <span>{move || format!("{} / {}", current_page.get() + 1, num_pages().max(1))}</span>
                    <button
                        on:click=move |_| if current_page.get() + 1 < num_pages() { set_current_page.update(|p| *p += 1); }
                        disabled=move || current_page.get() + 1 >= num_pages()
                        class="px-2 py-1 border border-border-primary rounded disabled:opacity-30"
                    >
                        "Next"
                    </button>
                </div>
            </div>
        </div>
    }
}

/// Trigger file download in browser
#[cfg(feature = "hydrate")]
fn trigger_download(content: &str, filename: &str, mime: &str) {
    use wasm_bindgen::JsCast;
    if let Some(window) = web_sys::window() {
        if let Some(doc) = window.document() {
            if let Ok(el) = doc.create_element("a") {
                let href = format!("data:{};charset=utf-8,{}", mime, js_sys::encode_uri_component(content));
                let _ = el.set_attribute("href", &href);
                let _ = el.set_attribute("download", filename);
                let _ = el.set_attribute("style", "display:none");
                if let Some(body) = doc.body() {
                    let _ = body.append_child(&el);
                    if let Some(html_el) = el.dyn_ref::<web_sys::HtmlElement>() {
                        html_el.click();
                    }
                    let _ = body.remove_child(&el);
                }
            }
        }
    }
}
```

- [ ] **Step 2: Add js-sys dependency for export URL encoding**

In `crates/daimon-app/Cargo.toml`, add to hydrate-only dependencies:
```toml
js-sys = { version = "0.3", optional = true }
```

And add `"dep:js-sys"` to the `hydrate` feature list.

- [ ] **Step 3: Register module in mod.rs**

Add to `crates/daimon-app/src/components/mod.rs`:
```rust
pub mod sortable_table;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p daimon-app --features ssr && cargo check -p daimon-app --features hydrate`
Expected: Both compile

- [ ] **Step 5: Commit**

```bash
git add crates/daimon-app/src/components/sortable_table.rs crates/daimon-app/src/components/mod.rs crates/daimon-app/Cargo.toml
git commit -m "Add SortableTable component with full UI — sort, search, paginate, export, column toggle"
```

### Task 8: SummaryBar and DetailLayout components

**Files:**
- Create: `crates/daimon-app/src/components/summary_bar.rs`
- Create: `crates/daimon-app/src/components/detail_layout.rs`
- Create: `crates/daimon-app/src/pages/cluster/agent_placeholder.rs`
- Modify: `crates/daimon-app/src/components/mod.rs`

- [ ] **Step 1: Create SummaryBar component**

Create `crates/daimon-app/src/components/summary_bar.rs`:

```rust
use leptos::prelude::*;
use crate::components::sparkline::Sparkline;

pub struct SummaryItem {
    pub label: &'static str,
    pub value: String,
    pub color: Option<String>,
    pub sparkline_data: Option<Vec<f64>>,
    pub sparkline_color: Option<String>,
}

#[component]
pub fn SummaryBar(items: Vec<SummaryItem>) -> impl IntoView {
    view! {
        <div class="flex gap-px bg-border-primary rounded-lg overflow-hidden mb-4">
            {items.into_iter().map(|item| {
                let color_class = item.color.clone().unwrap_or_default();
                view! {
                    <div class="flex-1 bg-surface-secondary p-3 text-center">
                        <div class="text-text-muted text-[10px] uppercase tracking-wider">{item.label}</div>
                        <div class="text-base font-bold mt-1" style=format!("color: {}", color_class)>
                            {item.value}
                        </div>
                        {item.sparkline_data.map(|data| {
                            let color = item.sparkline_color.unwrap_or_else(|| "#F59E0B".to_string());
                            view! {
                                <div class="mt-1 flex justify-center">
                                    <Sparkline data=data color=color width=80 height=20 />
                                </div>
                            }
                        })}
                    </div>
                }
            }).collect_view()}
        </div>
    }
}
```

- [ ] **Step 2: Create DetailLayout component**

Create `crates/daimon-app/src/components/detail_layout.rs`:

```rust
use leptos::prelude::*;
use leptos_router::components::{A, Outlet};
use leptos_router::hooks::use_location;

pub struct DetailTab {
    pub label: &'static str,
    pub path: String,
    pub requires_agent: bool,
}

#[component]
pub fn DetailLayout(
    title: String,
    #[prop(optional)]
    subtitle: Option<String>,
    tabs: Vec<DetailTab>,
    children: Children,
) -> impl IntoView {
    let location = use_location();
    let pathname = move || location.pathname.get();

    view! {
        <div>
            <div class="mb-4">
                <h1 class="text-xl font-semibold text-text-primary">{title}</h1>
                {subtitle.map(|s| view! { <p class="text-text-muted text-xs">{s}</p> })}
            </div>

            <div class="flex gap-1 border-b border-border-primary mb-4">
                {tabs.into_iter().map(|tab| {
                    let path = tab.path.clone();
                    let path2 = tab.path.clone();
                    view! {
                        <A
                            href=path
                            attr:class=move || format!(
                                "px-3 py-2 text-sm font-medium transition-colors -mb-px {}",
                                if pathname().ends_with(&path2) || pathname() == path2 {
                                    "text-accent-amber border-b-2 border-accent-amber"
                                } else {
                                    "text-text-muted hover:text-text-secondary"
                                }
                            )
                        >
                            {tab.label}
                        </A>
                    }
                }).collect_view()}
            </div>

            <div>{children()}</div>
        </div>
    }
}
```

- [ ] **Step 3: Create agent placeholder page component**

Create `crates/daimon-app/src/pages/cluster/agent_placeholder.rs`:

```rust
use leptos::prelude::*;

#[component]
pub fn AgentPlaceholder(
    /// Tab name (e.g., "Hardware", "RAID")
    tab_name: &'static str,
    /// Description of what data the agent provides
    description: &'static str,
) -> impl IntoView {
    view! {
        <div class="flex flex-col items-center justify-center min-h-[200px] text-text-muted">
            <div class="text-4xl mb-3">"ℹ"</div>
            <div class="text-sm font-semibold text-text-secondary">
                {format!("Install daimon-agent for {} data", tab_name)}
            </div>
            <div class="text-xs mt-2 max-w-md text-center leading-relaxed">
                {description}
            </div>
            <code class="mt-4 text-[11px] px-3 py-1.5 bg-surface-tertiary border border-border-primary rounded text-accent-amber">
                "curl -fsSL https://daimon.dev/install.sh | sh"
            </code>
        </div>
    }
}
```

- [ ] **Step 4: Register all new modules**

In `crates/daimon-app/src/components/mod.rs`:
```rust
pub mod summary_bar;
pub mod detail_layout;
```

In `crates/daimon-app/src/pages/cluster/mod.rs`, add:
```rust
pub mod agent_placeholder;
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p daimon-app --features ssr`
Expected: Compiles

- [ ] **Step 6: Commit**

```bash
git add crates/daimon-app/src/components/summary_bar.rs crates/daimon-app/src/components/detail_layout.rs crates/daimon-app/src/pages/cluster/agent_placeholder.rs crates/daimon-app/src/components/mod.rs crates/daimon-app/src/pages/cluster/mod.rs
git commit -m "Add SummaryBar, DetailLayout, and AgentPlaceholder components"
```

---

## Chunk 3: Table Refactor & Routes

Wire SortableTable into existing pages and add detail view routes.

### Task 9: Implement TableRow for NodeRow, GuestRow, StorageRow

**Files:**
- Modify: `crates/daimon-app/src/components/table.rs`

- [ ] **Step 1: Add node field to StorageRow for disambiguation**

In `crates/daimon-app/src/components/table.rs`, add `node: String` field to `StorageRow` struct (after `name`):

```rust
pub struct StorageRow {
    pub name: String,
    pub node: String,  // NEW — required for storage detail route
    pub storage_type: String,
    // ... rest unchanged
}
```

- [ ] **Step 2: Implement TableRow for NodeRow**

Add `use crate::components::sortable_table::{TableRow, ColumnDef, SortType};` at the top.

Implement the trait:

```rust
impl TableRow for NodeRow {
    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef { key: "name", label: "Node", sortable: true, default_hidden: false, sort_type: SortType::Text },
            ColumnDef { key: "status", label: "Status", sortable: true, default_hidden: false, sort_type: SortType::Text },
            ColumnDef { key: "cpu", label: "CPU", sortable: true, default_hidden: false, sort_type: SortType::Percentage },
            ColumnDef { key: "memory", label: "Memory", sortable: true, default_hidden: false, sort_type: SortType::Percentage },
            ColumnDef { key: "disk", label: "Disk", sortable: true, default_hidden: false, sort_type: SortType::Percentage },
            ColumnDef { key: "uptime", label: "Uptime", sortable: true, default_hidden: false, sort_type: SortType::Numeric },
        ]
    }

    fn cell_value(&self, col: &str) -> String {
        match col {
            "name" => self.name.clone(),
            "status" => self.status.clone(),
            "cpu" => format!("{:.1}", self.cpu_pct),
            "memory" => {
                if self.mem_total > 0 { format!("{:.1}", (self.mem_used as f64 / self.mem_total as f64) * 100.0) }
                else { "0.0".into() }
            }
            "disk" => {
                if self.disk_total > 0 { format!("{:.1}", (self.disk_used as f64 / self.disk_total as f64) * 100.0) }
                else { "0.0".into() }
            }
            "uptime" => self.uptime.to_string(),
            _ => String::new(),
        }
    }

    fn cell_view(&self, col: &str) -> AnyView {
        match col {
            "name" => view! { <span class="text-text-primary font-medium">{self.name.clone()}</span> }.into_any(),
            "status" => {
                let online = self.status == "online";
                view! {
                    <span class="inline-flex items-center gap-1.5 text-[12px]">
                        <span class=format!("w-2 h-2 rounded-full {}", if online { "bg-accent-green" } else { "bg-accent-danger" })></span>
                        {if online { "Online" } else { "Offline" }}
                    </span>
                }.into_any()
            }
            "cpu" => view! { {pct_bar(self.cpu_pct, "bg-accent-green")} }.into_any(),
            "memory" => {
                let pct = if self.mem_total > 0 { (self.mem_used as f64 / self.mem_total as f64) * 100.0 } else { 0.0 };
                view! {
                    <div>
                        {pct_bar(pct, "bg-accent-amber")}
                        <div class="text-text-muted text-[10px] mt-0.5">{format!("{} / {}", format_bytes(self.mem_used), format_bytes(self.mem_total))}</div>
                    </div>
                }.into_any()
            }
            "disk" => {
                let pct = if self.disk_total > 0 { (self.disk_used as f64 / self.disk_total as f64) * 100.0 } else { 0.0 };
                view! {
                    <div>
                        {pct_bar(pct, "bg-accent-purple")}
                        <div class="text-text-muted text-[10px] mt-0.5">{format!("{} / {}", format_bytes(self.disk_used), format_bytes(self.disk_total))}</div>
                    </div>
                }.into_any()
            }
            "uptime" => view! { <span class="text-text-secondary text-[13px]">{format_uptime(self.uptime)}</span> }.into_any(),
            _ => view! {}.into_any(),
        }
    }

    fn row_key(&self) -> String { self.name.clone() }

    fn row_link(&self) -> Option<String> {
        // Will be set dynamically by the page that knows the cluster_id
        None
    }
}
```

- [ ] **Step 3: Implement TableRow for GuestRow and StorageRow**

Follow the same pattern for `GuestRow` and `StorageRow`. Key differences:
- `GuestRow::row_key()` returns `vmid.to_string()`
- `StorageRow` columns include "Node" column
- `StorageRow::row_key()` returns `format!("{}:{}", self.node, self.name)`

- [ ] **Step 4: Update get_cluster_storage to populate node field**

In `crates/daimon-app/src/pages/cluster/detail.rs`, update the `get_cluster_storage` function to populate `StorageRow.node` from `PveResource.node`.

- [ ] **Step 5: Update page components to use SortableTable**

In `crates/daimon-app/src/pages/cluster/nodes.rs`, replace `NodeTable` with `SortableTable`:

```rust
use crate::components::sortable_table::SortableTable;
use crate::components::table::NodeRow;

// In the view:
Ok(rows) => view! { <SortableTable<NodeRow> rows=rows table_id="nodes" /> }.into_any(),
```

Do the same for `vms.rs`, `containers.rs`, `storage.rs`.

- [ ] **Step 6: Remove old table render components (NodeTable, GuestTable, StorageTable)**

Delete the `#[component] pub fn NodeTable`, `GuestTable`, `StorageTable` functions from `table.rs`. Keep the row structs and formatter functions — those are still used.

- [ ] **Step 7: Run tests and verify compilation**

Run: `cargo test -p daimon-app --lib`
Run: `cargo leptos build`
Expected: All tests pass, app compiles

- [ ] **Step 8: Commit**

```bash
git add crates/daimon-app/src/components/table.rs crates/daimon-app/src/pages/cluster/
git commit -m "Refactor tables to use SortableTable — sort, search, paginate, export on all tables"
```

### Task 10: Add detail view routes

**Files:**
- Modify: `crates/daimon-app/src/app.rs`
- Create: `crates/daimon-app/src/pages/cluster/node_detail.rs`
- Create: `crates/daimon-app/src/pages/cluster/node_charts.rs`
- Create: `crates/daimon-app/src/pages/cluster/vm_detail.rs`
- Create: `crates/daimon-app/src/pages/cluster/vm_charts.rs`
- Create: `crates/daimon-app/src/pages/cluster/lxc_detail.rs`
- Create: `crates/daimon-app/src/pages/cluster/lxc_charts.rs`
- Create: `crates/daimon-app/src/pages/cluster/storage_detail.rs`
- Create: `crates/daimon-app/src/pages/cluster/storage_usage.rs`
- Create: `crates/daimon-app/src/pages/cluster/storage_charts.rs`
- Modify: `crates/daimon-app/src/pages/cluster/mod.rs`

- [ ] **Step 1: Create stub detail page components**

Create each detail page file with a minimal skeleton that uses `DetailLayout` + `SummaryBar`. Each file follows this pattern:

```rust
// node_detail.rs
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use leptos_router::components::Outlet;
use crate::components::detail_layout::{DetailLayout, DetailTab};

#[component]
pub fn NodeDetail() -> impl IntoView {
    let params = use_params_map();
    let cluster_id = move || params.get().get("cluster_id").unwrap_or_default();
    let node_name = move || params.get().get("node_name").unwrap_or_default();

    let base = move || format!("/clusters/{}/nodes/{}", cluster_id(), node_name());

    view! {
        <DetailLayout
            title=node_name()
            subtitle=Some("PVE Node".to_string())
            tabs=vec![
                DetailTab { label: "Overview", path: base(), requires_agent: false },
                DetailTab { label: "Hardware", path: format!("{}/hardware", base()), requires_agent: true },
                DetailTab { label: "RAID", path: format!("{}/raid", base()), requires_agent: true },
                DetailTab { label: "Disks", path: format!("{}/disks", base()), requires_agent: true },
                DetailTab { label: "Storage", path: format!("{}/storage", base()), requires_agent: true },
                DetailTab { label: "Network", path: format!("{}/network", base()), requires_agent: true },
                DetailTab { label: "Services", path: format!("{}/services", base()), requires_agent: true },
                DetailTab { label: "Charts", path: format!("{}/charts", base()), requires_agent: false },
            ]
        >
            <Outlet />
        </DetailLayout>
    }
}
```

Create similar stubs for `VmDetail`, `LxcDetail`, `StorageDetail` with their respective tabs per spec.

Create placeholder chart pages (`node_charts.rs`, `vm_charts.rs`, `lxc_charts.rs`, `storage_charts.rs`) and `storage_usage.rs` with minimal "Coming soon" views.

- [ ] **Step 2: Register all new page modules**

Update `crates/daimon-app/src/pages/cluster/mod.rs` to include all new modules.

- [ ] **Step 3: Add routes to app.rs**

In `crates/daimon-app/src/app.rs`, add the detail view routes inside the existing cluster `ParentRoute`. Import all new page components. Add nested routes per the spec's routing section:

Detail routes go OUTSIDE the `ClusterDetail` ParentRoute (siblings, not children) to avoid double tab bars. The detail view fully replaces the cluster table view. Add a back link to return to the cluster tab.

```rust
// Inside the Layout ParentRoute, AFTER the existing cluster ParentRoute:

// Node detail (sibling to ClusterDetail, not nested inside it)
<ParentRoute path=(StaticSegment("clusters"), ParamSegment("cluster_id"), StaticSegment("nodes"), ParamSegment("node_name")) view=NodeDetail>
    <Route path=StaticSegment("") view=NodeOverview />
    <Route path=StaticSegment("hardware") view=|| view! { <AgentPlaceholder tab_name="Hardware" description="IPMI sensors, motherboard info, and BIOS details require the daimon-agent running on this node." /> } />
    <Route path=StaticSegment("raid") view=|| view! { <AgentPlaceholder tab_name="RAID" description="RAID controller, virtual drives, physical drives, cache policy, and BBU health." /> } />
    <Route path=StaticSegment("disks") view=|| view! { <AgentPlaceholder tab_name="Disks" description="Per-disk SMART data, temperature, utilization, and slot mapping." /> } />
    <Route path=StaticSegment("storage") view=|| view! { <AgentPlaceholder tab_name="Storage" description="Local mounts, NFS/CIFS/iSCSI/FC connections, and multipath status." /> } />
    <Route path=StaticSegment("network") view=|| view! { <AgentPlaceholder tab_name="Network" description="Per-NIC traffic, errors, drops, bond/bridge status." /> } />
    <Route path=StaticSegment("services") view=|| view! { <AgentPlaceholder tab_name="Services" description="PVE daemon states: pvedaemon, pveproxy, corosync, ceph services." /> } />
    <Route path=StaticSegment("charts") view=NodeCharts />
</ParentRoute>

// VM detail
<ParentRoute path=(StaticSegment("clusters"), ParamSegment("cluster_id"), StaticSegment("vms"), ParamSegment("vmid")) view=VmDetail>
    <Route path=StaticSegment("") view=VmOverview />
    <Route path=StaticSegment("processes") view=|| view! { <AgentPlaceholder tab_name="Processes" description="Top processes by CPU/RAM, zombie count, process list." /> } />
    <Route path=StaticSegment("services") view=|| view! { <AgentPlaceholder tab_name="Services" description="systemd units and Docker containers." /> } />
    <Route path=StaticSegment("network") view=|| view! { <AgentPlaceholder tab_name="Network" description="Listening ports, connections, per-interface traffic." /> } />
    <Route path=StaticSegment("logs") view=|| view! { <AgentPlaceholder tab_name="Logs" description="Recent journal entries, filterable by unit and priority." /> } />
    <Route path=StaticSegment("charts") view=VmCharts />
</ParentRoute>

// LXC detail (same structure as VM)
<ParentRoute path=(StaticSegment("clusters"), ParamSegment("cluster_id"), StaticSegment("containers"), ParamSegment("vmid")) view=LxcDetail>
    // ... same sub-routes as VM
</ParentRoute>

// Storage detail (includes node_name for disambiguation)
<ParentRoute path=(StaticSegment("clusters"), ParamSegment("cluster_id"), StaticSegment("nodes"), ParamSegment("node_name"), StaticSegment("storage"), ParamSegment("storage_name")) view=StorageDetail>
    <Route path=StaticSegment("") view=StorageOverview />
    <Route path=StaticSegment("devices") view=|| view! { <AgentPlaceholder tab_name="Devices" description="Per-disk status, temperature, and SMART data." /> } />
    <Route path=StaticSegment("usage") view=StorageUsage />
    <Route path=StaticSegment("charts") view=StorageCharts />
</ParentRoute>
```

- [ ] **Step 4: Verify all routes compile and navigate**

Run: `cargo leptos build`
Expected: Compiles. Navigate to `/clusters/{id}/nodes/{name}` shows detail layout with tabs.

- [ ] **Step 5: Commit**

```bash
git add crates/daimon-app/src/app.rs crates/daimon-app/src/pages/cluster/
git commit -m "Add detail view routes and page skeletons for nodes, VMs, LXCs, storage"
```

---

## Chunk 4: Detail View Content

Populate detail views with real PVE data — overview pages, summary bars, RRD charts.

### Task 11: Node detail overview with real data

**Files:**
- Modify: `crates/daimon-app/src/pages/cluster/node_detail.rs`
- Modify: `crates/daimon-app/src/pages/cluster/detail.rs` (add server functions)

- [ ] **Step 1: Add server functions for node detail data**

In `crates/daimon-app/src/pages/cluster/detail.rs`, add:

```rust
#[server]
pub async fn get_node_detail(cluster_id: String, node_name: String) -> Result<(NodeRow, Vec<GuestRow>), ServerFnError> {
    // Returns node status + list of VMs/LXCs on this node
    // Uses existing get_cluster_nodes + filtered get_cluster_vms/lxcs
}

#[server]
pub async fn get_node_rrd(cluster_id: String, node_name: String, timeframe: String) -> Result<Vec<daimon_pve::RrdDataPoint>, ServerFnError> {
    // Calls client.node_rrddata(node, timeframe)
    // Returns RRD data for sparklines and charts
}
```

- [ ] **Step 2: Build NodeOverview component**

Create the overview tab content (extracted from `node_detail.rs` or as a separate component). Includes:
- `SummaryBar` with Status, Uptime, CPU%, Memory%, PVE version, Kernel
- 4 chart panels using `Sparkline` (CPU, Memory, Net I/O, Disk I/O) with RRD data
- Guest list grid (clickable cards linking to VM/LXC detail)

- [ ] **Step 3: Verify against live PVE**

Run: `cargo leptos watch`
Navigate to a node detail page. Verify summary bar shows real data, charts render, guest list is accurate.

- [ ] **Step 4: Commit**

```bash
git commit -m "Node detail overview — summary bar, RRD charts, guest list with live PVE data"
```

### Task 12: VM and LXC detail overview with real data

**Files:**
- Modify: `crates/daimon-app/src/pages/cluster/vm_detail.rs`
- Modify: `crates/daimon-app/src/pages/cluster/lxc_detail.rs`
- Modify: `crates/daimon-app/src/pages/cluster/detail.rs` (add server functions)

- [ ] **Step 1: Add server functions for VM/LXC detail**

```rust
#[server]
pub async fn get_guest_detail(cluster_id: String, vmid: u32, guest_type: String) -> Result<(GuestDetailData), ServerFnError> {
    // Calls qemu_status/lxc_status + qemu_config/lxc_config
    // Returns merged status + config data
}

#[server]
pub async fn get_guest_rrd(cluster_id: String, node: String, vmid: u32, guest_type: String, timeframe: String) -> Result<Vec<daimon_pve::RrdDataPoint>, ServerFnError> {
    // Calls qemu_rrddata or lxc_rrddata
}
```

- [ ] **Step 2: Build VmOverview and LxcOverview components**

SummaryBar with: Status, Node, VMID, Uptime, CPU%, Memory (used/allocated), Disk (used/allocated).
4 chart panels: CPU, Memory, Net I/O, Disk I/O.
Config summary: cores, sockets, memory, disk devices, network devices.

- [ ] **Step 3: Verify and commit**

```bash
git commit -m "VM and LXC detail overview — status, config summary, RRD charts"
```

### Task 13: Storage detail overview and usage tab

**Files:**
- Modify: `crates/daimon-app/src/pages/cluster/storage_detail.rs`
- Modify: `crates/daimon-app/src/pages/cluster/storage_usage.rs`
- Modify: `crates/daimon-app/src/pages/cluster/detail.rs`

- [ ] **Step 1: Add server functions for storage detail**

```rust
#[server]
pub async fn get_storage_detail(cluster_id: String, node_name: String, storage_name: String) -> Result<StorageDetailData, ServerFnError> {
    // Returns storage info from cluster_resources filtered by node+storage
}

#[server]
pub async fn get_storage_rrd(cluster_id: String, node_name: String, storage_name: String, timeframe: String) -> Result<Vec<daimon_pve::RrdDataPoint>, ServerFnError> {
    // Calls storage_rrddata
}

#[server]
pub async fn get_storage_usage(cluster_id: String, node_name: String, storage_name: String) -> Result<Vec<StorageUsageRow>, ServerFnError> {
    // Lists VMs/LXCs that use this storage, with disk sizes
    // Parses VM/LXC configs to find which ones reference this storage
}
```

- [ ] **Step 2: Build StorageOverview and StorageUsage components**

Overview: SummaryBar (Type, Node, Content, Used/Total/Available, Status) + usage chart.
Usage tab: Table of VMs/LXCs using this storage pool.

- [ ] **Step 3: Verify and commit**

```bash
git commit -m "Storage detail — overview with usage chart, usage tab showing VM/LXC allocation"
```

### Task 14: Full-size chart pages with timeframe selector

**Files:**
- Modify: `crates/daimon-app/src/pages/cluster/node_charts.rs`
- Modify: `crates/daimon-app/src/pages/cluster/vm_charts.rs`
- Modify: `crates/daimon-app/src/pages/cluster/lxc_charts.rs`
- Modify: `crates/daimon-app/src/pages/cluster/storage_charts.rs`

- [ ] **Step 1: Build chart page component**

A reusable chart page pattern:
- Timeframe selector (Hour/Day/Week/Month/Year buttons)
- 4 large Sparkline panels (reusing Sparkline component at larger dimensions like 100%x120)
- Each chart labeled with metric name and current value

The chart pages all follow the same pattern with different RRD fetch functions.

- [ ] **Step 2: Verify all chart pages with live data**

Navigate to each entity's Charts tab. Select different timeframes. Verify data updates.

- [ ] **Step 3: Commit**

```bash
git commit -m "Full-size RRD chart pages with timeframe selector for all entity types"
```

---

## Chunk 5: WebSocket Real-Time Layer

Server-side PVE cache, WebSocket endpoint, and client-side auto-refresh.

### Task 15: WebSocket message types

**Files:**
- Create: `crates/daimon-app/src/ws.rs` (start with types only)

- [ ] **Step 1: Define WS message types**

Create `crates/daimon-app/src/ws.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsClientMsg {
    Subscribe { scope: WsScope },
    Unsubscribe { scope: WsScope },
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsServerMsg {
    Snapshot { scope: WsScope, data: serde_json::Value },
    Update { scope: WsScope, data: serde_json::Value },
    Pong,
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind")]
pub enum WsScope {
    ClusterResources { cluster_id: String },
    NodeRrd { cluster_id: String, node: String },
    GuestRrd { cluster_id: String, node: String, vmid: u32 },
    StorageRrd { cluster_id: String, node: String, storage: String },
}
```

- [ ] **Step 2: Write serialization tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_msg_subscribe_serializes() {
        let msg = WsClientMsg::Subscribe {
            scope: WsScope::ClusterResources { cluster_id: "abc".into() },
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"Subscribe\""));
        assert!(json.contains("\"kind\":\"ClusterResources\""));
    }

    #[test]
    fn server_msg_snapshot_serializes() {
        let msg = WsServerMsg::Snapshot {
            scope: WsScope::NodeRrd { cluster_id: "abc".into(), node: "pve1".into() },
            data: serde_json::json!([{"cpu": 0.5}]),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"Snapshot\""));
    }

    #[test]
    fn client_msg_ping_roundtrip() {
        let msg = WsClientMsg::Ping;
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: WsClientMsg = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, WsClientMsg::Ping));
    }

    #[test]
    fn ws_scope_equality() {
        let a = WsScope::ClusterResources { cluster_id: "x".into() };
        let b = WsScope::ClusterResources { cluster_id: "x".into() };
        assert_eq!(a, b);
    }
}
```

- [ ] **Step 3: Register module and run tests**

Add `#[cfg(feature = "ssr")] pub mod ws;` to `crates/daimon-app/src/lib.rs`.
Run: `cargo test -p daimon-app --lib --features ssr -- ws`
Expected: 4 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/daimon-app/src/ws.rs crates/daimon-app/src/lib.rs
git commit -m "Add WebSocket message types with serialization tests"
```

### Task 16: PVE cache and background polling

**Files:**
- Modify: `crates/daimon-app/src/state.rs`
- Modify: `crates/daimon-app/src/main.rs`

- [ ] **Step 1: Add PveCache struct to state**

In `crates/daimon-app/src/state.rs`, add:

```rust
#[cfg(feature = "ssr")]
pub struct PveCache {
    pub resources: std::collections::HashMap<String, Vec<daimon_pve::PveResource>>,
    pub node_rrd: std::collections::HashMap<(String, String), Vec<daimon_pve::RrdDataPoint>>,
    pub last_poll: std::collections::HashMap<String, std::time::Instant>,
}
```

Add to `AppState`:
```rust
pub pve_cache: Arc<tokio::sync::RwLock<PveCache>>,
pub ws_subscribers: Arc<tokio::sync::RwLock<WsSubscribers>>,  // for Task 17
pub poll_interval: tokio::sync::watch::Sender<u64>,
```

- [ ] **Step 2: Spawn background polling task in main.rs**

After `AppState` construction, spawn a `tokio::spawn` task that:
1. Receives interval updates via `watch::Receiver`
2. Polls each cluster's PVE API at the configured interval
3. Updates PveCache
4. Broadcasts changes to WebSocket subscribers (stub — filled in Task 17)

- [ ] **Step 3: Write PveCache change detection tests**

```rust
#[test]
fn cache_detects_changed_data() { /* compare serialized JSON */ }

#[test]
fn cache_ignores_unchanged_data() { /* same data = no update */ }
```

- [ ] **Step 4: Commit**

```bash
git commit -m "Add PVE cache with background polling loop and change detection"
```

### Task 17: WebSocket server endpoint

**Files:**
- Modify: `crates/daimon-app/src/ws.rs`
- Modify: `crates/daimon-app/src/main.rs`

- [ ] **Step 1: Implement ws_handler and subscription manager**

Add to `ws.rs`:
- `WsSubscribers` struct: HashMap of sender channels keyed by scope
- `ws_handler` function: Axum WebSocket handler that upgrades connection, reads Subscribe/Unsubscribe/Ping messages, sends Snapshot/Update/Pong responses
- `broadcast` function: sends Update to all subscribers matching a scope

- [ ] **Step 2: Register WS route in main.rs**

Add BEFORE the Leptos routes:
```rust
let app = Router::new()
    .route("/api/v1/ws", axum::routing::get(ws::ws_handler))
    .layer(axum::Extension(app_state.clone()))
    // ... then leptos_routes_with_context
```

- [ ] **Step 3: Test WebSocket connection manually**

Run: `cargo leptos watch`
Use `websocat ws://localhost:3000/api/v1/ws` to verify connection + Ping/Pong.

- [ ] **Step 4: Commit**

```bash
git commit -m "WebSocket server endpoint with subscription manager and broadcast"
```

### Task 18: Client-side auto-refresh component

**Files:**
- Create: `crates/daimon-app/src/components/auto_refresh.rs`
- Modify: `crates/daimon-app/src/components/mod.rs`

- [ ] **Step 1: Build AutoRefresh component**

Client-side component that:
- Opens WebSocket to `/api/v1/ws` on mount
- Sends Subscribe messages based on current page context
- Updates Leptos signals when Update messages arrive
- Falls back to HTTP polling if WS fails
- Shows interval selector dropdown with countdown indicator
- Exponential backoff on reconnect

- [ ] **Step 2: Register module**

Add `pub mod auto_refresh;` to `components/mod.rs`.

- [ ] **Step 3: Wire into table pages**

Add `AutoRefresh` component to each table page (nodes, vms, containers, storage). On update, refresh the Resource signal.

- [ ] **Step 4: Verify real-time updates**

Run: `cargo leptos watch`
Open tables. Verify data updates every 30s without page reload.

- [ ] **Step 5: Commit**

```bash
git commit -m "Auto-refresh component — WebSocket with polling fallback and interval selector"
```

### Task 19: Final integration and test suite

**Files:**
- All modified files

- [ ] **Step 1: Run full test suite**

Run: `cargo test --workspace`
Expected: ~45 tests pass (12 existing + ~33 new)

- [ ] **Step 2: Manual smoke test**

1. Login → Dashboard
2. Click cluster → Nodes table with sorting, search, pagination
3. Click node row → Node detail with summary bar, charts, guest list
4. Click Charts tab → Full RRD charts with timeframe selector
5. Click Hardware tab → Agent placeholder
6. Navigate to VM/LXC/Storage details → Same pattern
7. Toggle theme → Light mode works
8. Export CSV → File downloads
9. Auto-refresh → Data updates without reload

- [ ] **Step 3: Commit final integration**

```bash
git commit -m "Phase 5.5 complete — rich tables, detail views, sparklines, WebSocket, theme, export"
```

---

## Summary

| Chunk | Tasks | Estimated Tests | Key Deliverables |
|-------|-------|-----------------|-----------------|
| 1: PVE API | 1-3 | 9 | RRD types, 8 new client methods, dependency updates |
| 2: Components | 4-8 | 15 | Theme, Sparkline, SortableTable, SummaryBar, DetailLayout |
| 3: Table Refactor & Routes | 9-10 | 0 (existing tests cover) | All tables upgraded, detail view routes added |
| 4: Detail Content | 11-14 | 0 (manual testing) | Node/VM/LXC/Storage detail pages with live data |
| 5: WebSocket | 15-19 | 9 | WS types, server, cache, client auto-refresh |
| **Total** | **19 tasks** | **~33 new tests** | **Full Phase 5.5** |
