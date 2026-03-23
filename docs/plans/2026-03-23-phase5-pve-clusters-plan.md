# Phase 5: Multi-Cluster PVE Management — Implementation Plan

**Date**: 2026-03-23
**Status**: Implemented

## Tasks

### T1: PVE types (completed)
- Added PveNode, PveVm, PveLxc, PveStorage to daimon-pve/types.rs
- All types derive Debug, Clone, Deserialize with serde defaults

### T2: PVE client methods (completed)
- Added `from_token_string` constructor to parse `user@realm!tokenname=value`
- Added `nodes()`, `node_qemu()`, `node_lxc()`, `storage()` methods
- Tests: token string parsing with and without `!` separator

### T3: DB — clusters, preferences, role (completed)
- Added `clusters` and `user_preferences` tables to init_db
- Changed `find_user` to return 4-tuple with role
- Added: list_clusters, get_cluster, insert_cluster, delete_cluster
- Added: get_preference, set_preference
- Tests: cluster CRUD, unique name constraint, preference CRUD, role in find_user

### T4: Auth pipeline — role in Claims (completed)
- Added `role: String` field to Claims struct
- Updated `create_jwt` to accept `role: &str` parameter
- Updated tests to pass and assert role

### T5: AppState + dependencies (completed)
- Changed db from `Arc<Mutex>` to `Arc<tokio::sync::Mutex>`
- Added `pve_clients: Arc<RwLock<HashMap<String, Client>>>`
- Added daimon-pve and uuid to Cargo.toml with ssr feature
- Added Document, HtmlDocument to web-sys features

### T6: Reusable UI components (completed)
- TabBar: horizontal tabs with active amber underline
- MetricTable: name/sub + CPU bar + RAM bar + status dot
- UserMenu: avatar circle + dropdown with role + logout via cookie clear

### T7: Dynamic sidebar (completed)
- Replaced static CLUSTER_ITEMS with get_sidebar_clusters server fn
- Added "+ Add Cluster" link
- Section header changed to "PVE Clusters"

### T8: Global top bar + layout update (completed)
- Added header bar with UserMenu in Layout
- Auth check now returns (username, role) tuple
- Suspense wraps the authenticated layout

### T9: Cluster detail + server functions + add form (completed)
- ClusterDetail: info header + TabBar + Outlet
- Server functions: get_cluster_info, get_cluster_nodes, get_cluster_vms, get_cluster_lxcs, get_cluster_storage
- AddCluster: test_connection + save_cluster flow

### T10: Route restructure + settings (completed)
- ParentRoute for /clusters/:cluster_id with tab children
- Settings split into settings/mod.rs + settings/update.rs
- UpdateSection shows current version with placeholder check button
