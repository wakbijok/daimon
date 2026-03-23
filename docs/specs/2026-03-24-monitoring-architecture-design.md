# daimon Monitoring Architecture

## Overview

Full monitoring architecture for daimon — the data foundation that the AI engineer uses to observe, diagnose, and act. Six layers: collection (agent), storage (TSDB), observation (local engine), routing (alert triage), AI (cloud, token-conservative), and execution (agent commands).

Design principle: **local intelligence handles 99% of observation (zero tokens), cloud AI handles the 1% that needs reasoning (minimal tokens).**

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                        daimon-app (server)                      │
│                                                                 │
│  ┌──────────┐  ┌──────────────┐  ┌──────────┐  ┌────────────┐ │
│  │ PVE API  │  │ Observation  │  │  Alert   │  │ AI Engine  │ │
│  │ Client   │  │ Engine       │  │  Router  │  │ (cloud)    │ │
│  │          │  │ (local, no   │  │          │  │            │ │
│  │          │  │  tokens)     │  │          │  │ tokens     │ │
│  └────┬─────┘  └──────┬───────┘  └────┬─────┘  └─────┬──────┘ │
│       │               │               │              │         │
│  ┌────┴───────────────┴───────────────┴──────────────┴───────┐ │
│  │                    MetricsStore (TSDB)                     │ │
│  │              SQLite → redb → VictoriaMetrics               │ │
│  └────────────────────────────────────────────────────────────┘ │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                     Leptos UI                             │   │
│  │  Tables │ Detail Views │ Charts │ Alerts │ AI Chat       │   │
│  └──────────────────────────────────────────────────────────┘   │
└──────────────────────────┬──────────────────────────────────────┘
                           │ HTTP (push/pull/execute)
            ┌──────────────┼──────────────┐
            │              │              │
     ┌──────┴──────┐ ┌────┴─────┐  ┌─────┴──────┐
     │ daimon-agent │ │ daimon-  │  │ daimon-    │
     │ (host mode) │ │ agent    │  │ agent      │
     │ PVE node    │ │ (guest)  │  │ (guest)    │
     │             │ │ VM 100   │  │ LXC 200    │
     └─────────────┘ └──────────┘  └────────────┘
```

## Layer 1: Data Collection (daimon-agent)

### Agent Binary

Single Rust binary: `daimon-agent`. Auto-detects mode on startup:
- If `/etc/pve/` exists → host mode
- Otherwise → guest mode

Installation: download binary, run as systemd service. Self-registers with daimon-app on first start.

### Transport Protocol

Hybrid push/pull/execute over HTTP(S):

| Direction | Purpose | Protocol |
|---|---|---|
| Agent → daimon | Push metrics on interval (default 30s) | POST /api/v1/agent/metrics |
| daimon → Agent | Pull on-demand queries | GET /api/v1/query/{type} |
| daimon → Agent | Execute approved commands | POST /api/v1/execute |

Agent authenticates to daimon-app using a pre-shared token (generated during enrollment). All communication encrypted (TLS).

### Host Mode Collection

#### IPMI/BMC Sensors
- **Temperatures**: CPU, system, HDD, peripheral, PCH, per-DIMM (P1-DIMMA1, etc.)
- **Fans**: per-fan RPM, N/A detection (dead fan)
- **Voltages**: Vcpu, VDIMM, 12V, 5VCC, 3.3VCC, VBAT, 5V Dual, 3.3V AUX, 1.2V BMC, 1.05V PCH
- **PSU**: status, redundancy
- **Chassis**: intrusion detection
- **Source**: `ipmitool sdr` / `ipmitool sensor`
- **Status indicators**: green (normal), yellow (warning), red (critical), blue (N/A)

#### Motherboard
- Board: manufacturer, model, serial
- BIOS: vendor, version, date
- **Source**: `dmidecode`

#### RAID Controller
- Controller: model, firmware, driver version
- Virtual drives: name, RAID level, state, size, stripe size
- Physical drives: slot, model, serial, firmware, state, media type (SSD/HDD), size
- Cache policy: **WriteBack / WriteThrough** (critical for performance)
- Cache status: current WB/WT, dirty cache size
- BBU/CVM: health, charge level, learn cycle status, temperature
- Rebuild: progress, estimated time
- **Source**: `megacli` / `storcli` / `arcconf` / `ssacli` (auto-detected by controller type)

#### Physical Disks
- Per-disk identity: model, serial, firmware, slot position, device path (/dev/sdX)
- Slot-to-device mapping (which bay = which device)
- SMART: health assessment, temperature, power-on hours, reallocated sectors, pending sectors, uncorrectable errors
- Utilization: read/write IOPS, throughput
- **Source**: `smartctl`, `/sys/block/`, controller CLI for slot mapping

#### Memory
- DIMM inventory: slot, size, speed, type (DDR3/DDR4/DDR5), manufacturer
- Per-DIMM temperature (if available via IPMI)
- ECC: correctable/uncorrectable error counts
- **Source**: `dmidecode`, `edac-util`, IPMI

#### Network Interfaces
- Per-NIC: name, driver, link speed, link status, MAC
- Traffic: RX/TX bytes, packets, errors, drops per interface
- Bonds/bridges: member interfaces, mode, active slave
- **Source**: `/sys/class/net/`, `ethtool`, `/proc/net/dev`

#### Storage Mounts
- Local mounts: device, mountpoint, filesystem, usage, options
- NFS: server, export, mount status, latency
- CIFS/SMB: server, share, mount status
- iSCSI: target IQN, portal, session status, connection state
- FC: HBA model, WWN, port state, connected targets, LUN mapping
- Multipath: active paths, path states, policy
- **Source**: `/proc/mounts`, `iscsiadm`, `fcinfo`/`systool`, `multipathd`

#### PVE Services
- Service states: pvedaemon, pveproxy, pvestatd, pve-firewall, corosync, ceph-mon, ceph-osd, ceph-mds, spiceproxy
- **Source**: `systemctl` status checks

#### OS Basics
- Kernel version, PVE version
- Uptime, load average (1/5/15)
- Swap usage
- Filesystem usage per mount
- **Source**: `/proc/`, `uname`, `pvesh`

### Guest Mode Collection

#### Processes
- Top N by CPU usage, top N by RAM usage
- Total process count, running, sleeping, zombie count
- Per-process: PID, name, user, CPU%, MEM%, RSS, command
- **Source**: `/proc/`

#### Services
- systemd units: name, state (active/inactive/failed), sub-state
- Docker containers: name, image, state, CPU%, MEM, ports
- **Source**: `systemctl list-units`, Docker API/socket

#### Network
- Listening ports: port, protocol, PID, service name
- Established connections: count per remote, state distribution
- Per-interface traffic: RX/TX bytes, errors
- **Source**: `ss`, `/proc/net/`

#### Disk I/O
- Per-device: read/write IOPS, throughput (MB/s), latency (avg, p99)
- **Source**: `/proc/diskstats`, `/sys/block/`

#### Logs
- journald: last N entries, filterable by unit/priority
- Configurable log paths: tail + pattern match for keywords (ERROR, WARN, FATAL)
- **Source**: `journalctl`, file read

#### Memory
- Total, used, free, available, buffers, cached, swap used
- Per-process top consumers
- **Source**: `/proc/meminfo`

#### Custom Checks (user-defined)
- HTTP health: URL, expected status, response time
- TCP port: host:port, open/closed, latency
- Script: run user script, capture exit code + stdout
- DNS: resolve hostname, expected IP
- Certificate: check TLS cert expiry

## Layer 2: Data Storage (MetricsStore)

### Interface

```rust
trait MetricsStore: Send + Sync {
    async fn write(&self, host: &str, timestamp: u64, metrics: &[Metric]) -> Result<()>;
    async fn query_latest(&self, host: &str, metric_name: &str) -> Result<Option<DataPoint>>;
    async fn query_range(&self, host: &str, metric_name: &str, start: u64, end: u64, step: u64) -> Result<Vec<DataPoint>>;
    async fn query_hosts(&self) -> Result<Vec<String>>;
    async fn delete_older_than(&self, age_secs: u64) -> Result<u64>;
    async fn migrate_from(&self, source: &dyn MetricsStore) -> Result<MigrationStats>;
}
```

### Backends

| Backend | Scale | Embedded? | When |
|---|---|---|---|
| SQLite | 1-50 hosts | Yes | Phase 6 (first) |
| redb | 50-200 hosts | Yes | Phase 7+ |
| VictoriaMetrics | 200+ hosts | No (sidecar) | Phase 8+ |

### Retention

Default retention policies (configurable):
- Raw (30s): 7 days
- 5-min aggregated: 30 days
- 1-hour aggregated: 1 year
- 1-day aggregated: 5 years

### Migration

```bash
daimon migrate-metrics --from sqlite --to redb
```

Built-in CLI command. Reads all from source, writes to target. Zero data loss.

## Layer 3: Observation Engine (local, zero tokens)

Continuous loop running inside daimon-app. Processes every metric as it arrives from agents. Entirely local computation — no cloud API calls, no token cost.

### Detection Methods

| Method | How it works | Example |
|---|---|---|
| **Static thresholds** | Value > limit | Disk temp > 60°C |
| **Rate thresholds** | Change rate > limit | Disk usage growing > 5GB/day |
| **Moving average** | Compare to N-period average | CPU avg this hour is 2x the 7-day average |
| **Z-score anomaly** | Standard deviations from baseline | Network traffic 4σ above normal for this time of day |
| **Missing data** | Expected metric not received | Agent heartbeat missing for 2 minutes |
| **Change detection** | Periodic snapshot diff | New listening port, service stopped, mount disappeared |
| **Rule correlation** | If A and B within time window | Fan dead + adjacent disk hot within 10 minutes |
| **Pattern matching** | Known bad patterns | RAID rebuild + high I/O = known risk |

### Baseline Learning

- First 7 days: learning mode (no anomaly alerts, only static thresholds)
- After 7 days: anomaly detection activates using collected baseline
- Baselines are per-host, per-metric, per-time-of-day (weekday vs weekend)
- Stored in MetricsStore as aggregated statistics

### Observation Output

Structured alert objects:

```rust
struct Observation {
    id: String,
    timestamp: u64,
    host: String,
    severity: Severity,        // Info, Warning, Critical
    category: String,          // thermal, storage, network, service, etc.
    title: String,             // "Thermal anomaly on nargothrond"
    summary: String,           // Pre-processed context for AI
    related_metrics: Vec<MetricSnapshot>,
    correlation_ids: Vec<String>, // Linked observations
    suggested_action: Option<String>, // If pattern is known
}
```

## Layer 4: Alert Router

Decides what happens with each observation. Runs locally, no tokens.

| Rule | Action | Tokens |
|---|---|---|
| Info severity | Log + display in UI timeline | Zero |
| Warning severity | Notify user (UI badge + optional webhook) | Zero |
| Critical + known pattern | Auto-propose known fix → human approval | Zero |
| Critical + unknown pattern | Escalate to AI Engine | **Yes** |
| Agent heartbeat lost | Mark host as unreachable in UI | Zero |
| Auto-resolve pattern | Apply fix, log action, close | Zero |

### User-configurable

- Severity thresholds per metric (override defaults)
- Notification channels: UI, webhook, email (future)
- AI escalation policy: always, business hours only, manual only
- Auto-resolve rules: enable/disable per pattern

## Layer 5: AI Engine (cloud, token-conservative)

Only invoked by Alert Router for Critical + unknown pattern alerts. Receives pre-processed `Observation` summary (~100-200 tokens input).

### AI Workflow

1. Receive observation summary from Alert Router
2. Optionally pull additional data from agents (on-demand queries)
3. Consult knowledge base (past similar incidents + resolutions)
4. Generate diagnosis + proposed remediation
5. Submit proposal for human approval
6. On approval: send structured commands to agent for execution
7. Verify fix (agent confirms, metrics return to normal)
8. Store resolution in knowledge base for future pattern matching

### Token Budget

| Step | Estimated tokens |
|---|---|
| Observation summary (input) | 100-200 |
| Additional context pulls | 100-300 |
| AI diagnosis + proposal (output) | 200-500 |
| **Total per incident** | **~400-1000** |

At ~$0.015/1K tokens (Claude Sonnet): ~$0.006-0.015 per incident.

### Knowledge Base

- Stored locally in SQLite
- Past incidents: observation → diagnosis → fix → outcome
- Pattern library: known failure patterns + proven remediations
- Grows over time — AI gets smarter without more tokens

## Layer 6: Agent Execution

Commands flow: AI proposal → human approval → daimon-app → agent → execute → report back.

### Command Types

| Type | Example | Requires approval |
|---|---|---|
| **Query** | "List top processes" | No (read-only) |
| **Service** | "Restart nginx" | Yes |
| **Config** | "Set fan mode to Optimal" | Yes |
| **System** | "Clear disk cache" | Yes |
| **Dangerous** | "Kill PID", "Reboot" | Yes + confirmation |

### Command Blocklist

Agent refuses certain commands regardless of approval:
- `rm -rf /`
- `dd if=/dev/zero`
- `mkfs` on mounted filesystems
- Format/wipe operations

### Execution Logging

Every command: timestamp, who approved, what was sent, agent output, success/failure. Full audit trail in SQLite.

## Detail Views (click-through UI)

### Node Detail

| Tab | Data | Source |
|---|---|---|
| **Overview** | Status, uptime, PVE version, kernel, load avg, summary charts | PVE API + agent |
| **Hardware** | IPMI sensors (full table: status/sensor/reading), motherboard, BIOS | Agent (host) |
| **RAID** | Controller, VDs, PDs, cache policy (WB/WT), BBU health | Agent (host) |
| **Disks** | Per-disk: model, serial, slot, temp, SMART, utilization | Agent (host) |
| **Storage** | Mounts (local/NFS/CIFS/iSCSI/FC), external connectivity, multipath | Agent (host) |
| **Network** | Per-NIC traffic, errors, bond status | Agent (host) |
| **Services** | PVE daemon states, corosync, ceph | Agent (host) |
| **Charts** | CPU, RAM, disk, network over time | TSDB |

### VM/LXC Detail

| Tab | Data | Source |
|---|---|---|
| **Overview** | Allocated vs actual, status, node, uptime, summary charts | PVE API |
| **Processes** | Top by CPU/RAM, zombie count | Agent (guest) |
| **Services** | systemd units, Docker containers | Agent (guest) |
| **Network** | Listening ports, connections, per-interface traffic | Agent (guest) |
| **Logs** | Recent log entries, filterable | Agent (guest) |
| **Charts** | CPU, RAM, network, disk I/O over time | TSDB |

Tabs requiring agent show: `"ℹ Install daimon-agent for [tab name] data"` when agent not present.

### Storage Detail

| Section | Data | Source |
|---|---|---|
| **Summary** | Total/used/available, pool type | PVE API |
| **Devices** | Per-disk: status, temp, SMART, utilization | Agent (host) |
| **VM usage** | Which VMs/LXCs use this pool, how much each | PVE API |
| **Performance** | I/O rates, latency | Agent (host) / TSDB |

## UI Features (table improvements)

All table views include:
- **Column sorting**: click header to sort asc/desc
- **Search/filter**: text input filters across all visible fields
- **Auto-refresh**: configurable interval (5s/15s/30s/60s/off)
- **Row click**: navigates to detail view

## Implementation Phases

| Phase | What | Depends on |
|---|---|---|
| **Phase 5.5** | Richer PVE API data + sorting + search + PVE RRD sparklines | Current code |
| **Phase 6** | daimon-agent binary (host+guest modes), MetricsStore trait + SQLite backend, push endpoint | Phase 5 |
| **Phase 7** | Observation Engine (thresholds + trend detection), alert UI | Phase 6 |
| **Phase 8** | AI Engine integration (cloud API), incident pipeline, approval flow, knowledge base | Phase 7 |
| **Phase 9** | Agent execution (command types, blocklist, audit), on-demand pull queries | Phase 8 |
| **Phase 10** | Baseline learning, anomaly detection, rule correlation | Phase 7 data |

Each phase produces working, testable software. The system is useful from Phase 6 (agent + metrics) even without AI.

## Design Decisions

- **One agent binary, two modes**: auto-detect host vs guest. Simpler distribution and updates.
- **Hybrid transport**: push (heartbeat/metrics), pull (on-demand), execute (approved commands). All three needed for agentic AI.
- **Local observation, cloud AI**: 99% of monitoring is local computation (zero tokens). AI only invoked for complex unknowns.
- **Pre-processed alerts**: AI receives ~200 token summaries, not raw metric streams. Token-conservative by design.
- **MetricsStore trait**: pluggable backends with built-in migration. Start SQLite, scale to redb or VictoriaMetrics.
- **Knowledge base**: AI learns from past incidents. Gets smarter without more tokens over time.
- **Command blocklist**: agent refuses dangerous operations regardless of approval. Safety by default.
- **RAID cache visibility**: WriteBack/WriteThrough status is a first-class metric. Wrong cache policy can cripple performance.
- **Agent replaces SSH**: structured commands through agent instead of raw SSH. Better logging, safer, auditable.
