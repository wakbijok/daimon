# daimon

![status](https://img.shields.io/badge/status-in%20active%20development-orange)
![license](https://img.shields.io/badge/license-MIT-blue)
![rust](https://img.shields.io/badge/rust-2024%20edition-dea584)

Multi-agent system for managing infrastructure.

daimon is a Rust workspace that runs an orchestrator + worker agents over a
versioned capability bus, gated by a policy engine + kill switch + operator
approval inbox. It reaches managed infrastructure agentlessly over SSH,
REST, and SNMP. State lives in a five-tier storage backend (relational +
vector + KV + time-series + graph). Memory is hybrid-RAG with cross-encoder
rerank. Observability pulls Prometheus into a dedicated time-series tier
and emits anomaly events back onto the bus.

> **In active development.** APIs and storage shapes are still moving. Not
> ready for production use.

## What it does

- **Orchestrator** plans operator intents — LLM-emitted DAGs validated
  against the capability registry, executed topologically with
  `depends_on` barriers + replan-on-failure + operator-approval
  escalation on failure. Plans persist in both Postgres (canonical) and
  the graph tier (for cross-reference + blast-radius queries).
- **Worker agents** execute capabilities against real infrastructure
  over SSH / REST / SNMP. First two workers:
  - `tool-network` — RouterOS over SSH. Read capabilities (system info,
    interfaces, IPs, firewall list) plus guarded write capabilities
    (firewall drop-rule add/remove, SSH key import/remove) with input
    validation against a strict allowlist character set.
  - `tool-platform` — Proxmox VE driver implementing the generic
    `Platform` trait (snapshot/clone capability slots ready, write side
    is a future increment).
- **Guard** — TOML policy DSL with glob matching, in-process approval
  queue, and a kill switch (file watcher at `/var/lib/daimon/KILL` +
  `SIGUSR1`, manual `rm` to resume).
- **Operator approval inbox** at `/admin/approvals` — every pending
  write surfaces with a blast-radius summary drawn from the graph tier
  (what depends on this target, what plans touch it, which credentials
  are in scope). Inline approve / deny + audit trail.
- **Memory + RAG** — relational canonical content tier + vector store
  with hybrid retrieval (dense + sparse with reciprocal-rank fusion)
  + cross-encoder rerank + greedy-MMR context packer with a token
  budget. A separate working-memory tier holds recent conversation
  turns, a KV scratchpad, and per-agent task queues.
- **Observer** — Prometheus ingest (PromQL instant + range) writes
  normalised metrics to a dedicated time-series tier. A TOML-defined
  named-query library covers the operator-curated default set. Anomaly
  detectors emit events to both storage and the bus.
- **Vault** — in-tree credential vault. Per-row XChaCha20-Poly1305 with
  the master key loaded via systemd `LoadCredentialEncrypted=`. A KMS
  abstraction is in place (local-file + HashiCorp Vault Transit
  available; AWS KMS + PKCS#11 stubbed).
- **Audit** — append-only hash-chained log. A database trigger computes
  the prev-hash linkage on insert; updates and deletes are blocked. A
  separate CLI snapshots chain heads to a sidecar table + file mirror
  for tamper-evidence.
- **Multi-tenant isolation** — row-level security on every
  tenant-scoped table. A dedicated integration test proves
  cross-tenant credential reveal is blocked and per-tenant audit hash
  chains stay independent.
- **Settings + self-update** — `/settings` is a 9-tab operator surface
  (identity, connections, LLM providers, guard, observer, RAG, vault,
  system, update). The Update tab picks a release channel (stable from
  GitHub, beta from GitLab), checks the matching release API, and
  writes a flag file that a systemd path-unit watches to swap the
  binary + restart agents with automatic rollback on boot failure.

## Architectural choices worth knowing

| Choice | What it means |
| --- | --- |
| Broker pattern | Worker agents never see credentials. A broker resolves the credential reference, dispatches over a transport, and zeroizes after use. The keystone is enforced by a test that the worker crates cannot import the vault or inventory crates. |
| Capability versioning | Every action a worker exposes is a `(name, SemVer)` tuple. Plans reference capabilities by version requirement; the registry resolves to the actual provider. Agents can roll forward or back without breaking plans in flight. |
| Append-only audit hash chain | Every state-changing action emits an audit event. The DB computes the prev-hash linkage. Heads are anchored periodically for external tamper-evidence. |
| In-tree vault | Credentials live inside daimon. No external secrets manager required for the default deployment. KMS pluggable. |
| Five-tier storage | Relational (Postgres) + vector (Qdrant) + KV (Redis) + time-series (VictoriaMetrics) + graph (NornicDB). Each tier is chosen for its access pattern; none is forced to do another's job. |
| Operator kill switch | No agent can override it. Resume is manual; there's no auto-resume. |

## Crates

```
crates/
├── daimon-core            agent trait + capability registry + envelopes
├── daimon-runtime         in-proc bus + supervisor (restart-on-panic) + optional NATS bus
├── daimon-vault           in-tree credential vault (Postgres + XChaCha20)
├── daimon-inventory       target registry (Postgres + in-memory)
├── daimon-transport       Transport trait + russh impl + stubs
├── daimon-broker          PUBLIC action surface (the credential-boundary keystone)
├── daimon-audit           append-only hash-chained audit log
├── daimon-anchor          audit chain snapshot/verify CLI
├── daimon-db              schema migrations + Pool
├── daimon-kms             KMS abstraction (local-file / Vault Transit / stubs)
├── daimon-memory          long-term memory tier (Postgres canonical content)
├── daimon-rag             hybrid RAG (dense + sparse + rerank + packer)
├── daimon-redis           working memory tier (Redis + in-proc fallback)
├── daimon-graph           graph tier (NornicDB over Cypher) — plan + blast-radius
├── daimon-llm             multi-provider client (Anthropic / OpenAI / Ollama)
├── daimon-guard           policy engine + kill switch + approval queue
├── daimon-orchestrator    plan persistence + DAG executor + LLM plans
├── daimon-observer        Prometheus ingest + named queries + anomalies
├── daimon-tool-network    first worker agent — RouterOS over SSH
├── daimon-tool-platform   Platform trait + PVE driver
├── daimon-pve             Proxmox VE REST client
├── daimon-app             Leptos SSR + WASM hydrate (operator UI)
├── daimon-cli             demo + ingest + retrieve CLIs
└── daimon-migrate         one-shot SQLite → Postgres bridge (transitional)
```

## Development

### One-time setup

```sh
brew install just postgresql@16 redis victoriametrics nats-server
cargo install cargo-leptos --locked
cargo install cargo-zigbuild   # musl linker on macOS, for `just build`
rustup target add wasm32-unknown-unknown
just qdrant-install            # downloads Qdrant native binary
just nornicdb-install          # clones + go-builds NornicDB at ~/.daimon/bin/
```

`nornicdb-install` requires the Go toolchain (`brew install go`). The
build uses `-tags noui,nolocalllm` so it doesn't need a pre-built npm
dist or the bundled llama.cpp library.

### Daily flow

```sh
just pg-up && just pg-create-db && just pg-migrate    # relational
just qdrant-up                                         # vector
just redis-up                                          # working memory
just vm-up                                             # time-series
just nornicdb-up                                       # graph
just nats-up                                           # bus (only for multi-process deployments)
just dev                                               # Leptos dev :3030
```

The full daimon experience needs all six daemons. Most pages still
render with backends offline — the operator UI degrades gracefully and
the System tab in `/settings` shows live reachability per backend.

`just dev` overrides the workspace `bin-target-triple` (musl) with the
host triple so the binary runs on macOS. `just build` honours the musl
config and produces a static Linux x86_64 bin under
`target/x86_64-unknown-linux-musl/release/`.

### Recipes

| Command | What |
| --- | --- |
| `just` | List all recipes |
| `just dev` / `just dev-port 3030` | Dev server (http://127.0.0.1:3030) |
| `just check` | SSR + hydrate compile check |
| `just test` | Full workspace tests |
| `just test-broker` | Broker keystone tests (credential boundary + audit invariants) |
| `just test-isolation` | Multi-tenant isolation e2e (live Postgres) |
| `just test-rag` | Hybrid RAG e2e — first run downloads ~250 MB models |
| `just build` | Release musl bin |
| `just promote` | Promote staging HEAD to production (preconditioned) |
| `just keygen` | Generate `/tmp/daimon-dev.key` (auto-invoked by `just dev`) |
| `just dev-reset` | DESTRUCTIVE — wipe local vault / inventory / audit + master key |
| `just dev-reset-admin` | Re-seed admin/devadmin on next `just dev` |
| `just pg-up` / `pg-down` / `pg-status` / `pg-psql` | Postgres lifecycle |
| `just pg-migrate` / `pg-create-db` / `pg-drop-db` | DB schema lifecycle |
| `just pg-reset-tenant <slug>` | Wipe tenant data (keeps the tenant row) |
| `just migrate-data` / `migrate-data-verify` | One-shot SQLite → Postgres |
| `just qdrant-up` / `qdrant-down` / `qdrant-status` / `qdrant-reset` | Qdrant lifecycle |
| `just redis-up` / `redis-down` / `redis-status` | Redis lifecycle |
| `just vm-up` / `vm-down` / `vm-status` / `vm-reset` | VictoriaMetrics lifecycle |
| `just nornicdb-up` / `nornicdb-down` / `nornicdb-status` / `nornicdb-reset` | NornicDB lifecycle |
| `just nats-up` / `nats-down` / `nats-status` / `nats-reset` | NATS sidecar lifecycle |
| `just audit-snapshot <tenant>` / `audit-verify` / `audit-anchors` | Audit chain ops |
| `just status` | Repo hygiene snapshot |

### Daemons

Native dev — no Docker. Six daemons run as native processes:

| Service | Port | Tier | Data |
| --- | --- | --- | --- |
| Postgres 16 | 5432 | Relational (canonical) | brew default data dir |
| Qdrant | 6333 REST / 6334 gRPC | Vector | `.qdrant-data/` |
| Redis 7 | 6379 | KV / working memory | `~/.daimon/redis-data/` |
| VictoriaMetrics | 8428 | Time-series | `.victoria-metrics-data/` |
| NornicDB | 7474 HTTP / 7687 Bolt | Graph | `.nornicdb-data/` |
| NATS | 4222 client / 8222 monitoring | Inter-agent bus | `.nats-data/` |

NATS is only required when running per-agent processes (multi-process
deployments). Single-process dev uses the in-process bus and can leave
NATS off.

### Production deployment

`deploy/systemd/` ships unit templates for a per-agent deployment:

- `daimon-nats.service` — bus sidecar
- `daimon-agent@.service` — per-agent template (`systemctl enable
  daimon-agent@tool-network.service` etc.)
- `daimon-update.path` + `daimon-update.service` +
  `daimon-update-hook.sh` — Update-tab driven self-update with binary
  swap, agent restart, and rollback-on-boot-failure

See `deploy/systemd/README.md` for the install + boot order.

## Reading the codebase

Start with `crates/daimon-core` (agent trait + capability registry +
envelopes) and `crates/daimon-broker` (the credential-boundary keystone —
workers never see raw credentials). The `Broker::execute` flow is the
canonical action path: inventory → vault → transport → audit. Tests in
`crates/daimon-broker/tests/agent_never_sees_credential.rs` prove the
invariant.

For the operator UI, `crates/daimon-app/src/app.rs` is the route table.
Server-fns live alongside their pages (`admin_*.rs`); WASM-side
components live in `components/`. The chat surface is a floating bubble
mounted in `components/layout.rs` so it survives route changes. Notable
admin surfaces: `/admin/approvals` (operator inbox with blast-radius),
`/admin/plans` (plan inspector with DAG view), `/admin/observer`
(metrics + anomalies, VM-backed), and `/settings` (9-tab system
configuration including the Update tab).

The end-to-end flow for a write action — chat intent → orchestrator
plan → guard approval → broker dispatch → audit chain — is captured by
`crates/daimon-tool-network/tests/tiktok_block_vertical.rs`. Run it
against StubTransport with `cargo test -p daimon-tool-network --test
tiktok_block_vertical`.

## License

MIT
