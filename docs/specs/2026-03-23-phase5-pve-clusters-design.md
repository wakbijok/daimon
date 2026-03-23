# Phase 5: Multi-Cluster PVE Management — Design Spec

**Date**: 2026-03-23
**Status**: Implemented

## Overview

Phase 5 extends daimon from a single-cluster placeholder UI to a fully functional multi-cluster Proxmox VE management interface. Users can add, remove, and switch between PVE clusters. Each cluster shows live node, VM, container, and storage data via the PVE API.

## Architecture Decisions

### Multi-Cluster State

- Clusters stored in SQLite `clusters` table (id, name, api_url, token, notes)
- PVE clients loaded at startup into `AppState.pve_clients` (RwLock<HashMap>)
- New clusters added at runtime update both DB and in-memory map

### Auth Pipeline Changes

- `Claims` now includes `role` field for RBAC groundwork
- `find_user` returns 4-tuple: (id, username, password_hash, role)
- JWT carries role; auth_guard returns `(username, role)` tuple

### Routing

- `/clusters/add` — add new cluster form
- `/clusters/:cluster_id/nodes|vms|containers|storage` — tabbed detail views
- ParentRoute with ClusterDetail provides tab bar + outlet

### UI Components

- `TabBar` — horizontal tab navigation with active amber underline
- `MetricTable` — reusable table with CPU/RAM mini bars and status dots
- `UserMenu` — avatar + dropdown with role display and logout
- `Sidebar` — dynamic cluster list from server function

### Database Schema Additions

- `clusters` table with UNIQUE name constraint
- `user_preferences` table for per-user settings (composite PK: user_id + key)

## Data Flow

1. Startup: `init_db` creates tables, loads clusters, builds PVE clients
2. Sidebar: `get_sidebar_clusters` server fn queries DB
3. Cluster detail: server fns read from `pve_clients` RwLock
4. Add cluster: test connection, save to DB, insert into RwLock map
