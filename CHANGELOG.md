# Changelog

All notable changes to daimon. Versions follow the revival roadmap (P0–P7).

## [0.9.0] — 2026-07-04

First tagged release of the revived, single-organization, platform-agnostic
daimon AIOps platform. Feature-complete against the revival SRS; the `1.0.0`
tag is reserved until release hardening (signing / SBOM / container image /
release automation) lands. Production target: fully static musl binary + the
dmem HTTP sidecar for long-term memory (keyword-only recall by default).

### P0 — boot & scaffold
Single-org de-tenanting (dropped multi-tenancy, RLS, clusters, PVE); global
hash-chained audit; self-consistent `daimon.service` + installer; CI gate
(migrate replay + ssr/hydrate compile).

### P1 — IAM & auth
Multi-user IAM (four roles), password login, authenticated `/api/v1/ws`
upgrade, server-side read-only derivation (no caller-supplied privilege), boot
policy-coherence linter (every write resolves to deny/require_approval).

### P2 — harness & connectors
Multi-agent harness (in-process bus, capability registry, supervisor); chat +
orchestrator dispatch routed through the bus; saga rollback via compensating
capabilities; the declarative `ConnectorDriver` (`.toml` profiles) beside code
drivers — by-capability routing across transports.

### P3 — AIOps loop
Long-term memory as the dmem sidecar (behind `MemoryService`); observer
anomaly → supervised triage → persisted (not auto-run) plan; self-observability
(`/healthz`, `/metrics`, tracing). End-to-end remediation vertical proven.

### P4 — messaging gateways
Transport-agnostic `ReplySink`; the `daimon-gateway` crate; Telegram (webhook +
long-poll) and Matrix (/sync) adapters; fail-closed identity binding; the
Channels settings tab. Same Harness/Guard/audit path as a browser turn.

### P5 — connector expansion
Per-connector auth schemes (Bearer / header / none); Proxmox (VMs), generic
Linux/mini-PC over SSH, and read-only SNMP transports; skills as declarative
plan templates. Compute now spans baremetal + virtualization + cloud.

### P6 — settings coherence + alert routing
`ConfigResolver` (DB `app_config` → env → default, hot-reload); server-side
vault interception (secrets → `vault://` refs, never plaintext); live-tunable
guard/observer parameters; consumed-key registry + boot config-coherence lint;
Targets/Connectors + IAM settings tabs. Outbound alert routing to Telegram/Matrix
(fail-closed recipients, fail-soft delivery, dedup) + approve-over-chat.

### P7 — console & release
Guard→bus `AwaitingApproval` emit (completes alert routing); persisted reply-to
correlation for approve-over-chat; durable, owner-scoped chat history in Postgres
(V029) with retention independent of the auth TTL; server-side-enforced
model/effort picker; real Dashboard + Incidents surfaces; code-derived,
drift-checked config reference + operator runbook (incl. master-key escrow); this
release cut.

### Security invariants (maintained throughout)
D21 credential boundary (vault only via the broker); `param::validate` injection
chokepoint; secrets by `vault://` reference; server-side authority checks;
fail-closed gateway identity; the operator-only kill switch.
