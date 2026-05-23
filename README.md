# daimon

Multi-agent AIOps system for regulated infrastructure.

daimon is a Rust workspace that runs an orchestrator + worker agents over a
capability-versioned bus, gated by a policy engine + kill switch + approval
inbox, backed by a hybrid-RAG memory tier and an observer that ingests
PromQL/LogQL. The default target is multi-tenant banking-grade SaaS (Bank
Negara Malaysia RMiT crosswalk under `daimon-docs/compliance/`), with a
homelab-friendly single-tenant mode for development.

> **Status: pre-production.** Phases 0–7 shipped. Phase 8 (locked vertical
> demo + NATS sidecar + per-agent systemd units + kill-switch e2e) lands
> end-of-May 2026. Not yet ready for paying customers.

## What it does

- **Orchestrator** plans operator intents (LLM-emitted DAGs validated against
  the capability registry; topological executor with `depends_on` barriers,
  replan-on-failure, approval-gated escalation).
- **Worker agents** execute capabilities against real infrastructure —
  agentless via SSH / REST / SNMP. First two workers:
  - `tool-network` — read + (guarded) write MikroTik over RouterOS CLI
  - `tool-platform` — Proxmox VE driver implementing the generic `Platform`
    trait (snapshot/clone capability slots ready, write side lands in 7.1)
- **Guard** — TOML policy DSL + glob matching + kill switch (file watcher at
  `/var/lib/daimon/KILL` + `SIGUSR1`, manual `rm` to resume) + in-process
  approval queue surfaced at `/admin/approvals`.
- **Memory + RAG** — Postgres canonical content tier + Qdrant hybrid
  (dense BGE-small + SPLADE++ sparse, RRF fusion via the Query API) + BGE
  cross-encoder rerank + greedy-MMR context packer with token budget. Redis
  working memory tier (recent conversation turns, KV scratchpad, per-agent
  task queue).
- **Observer** — Prometheus ingest (PromQL instant + range queries),
  TOML-defined named-query library, anomaly emitter writing to
  `observer.anomalies` + the bus.
- **Vault** — in-tree, batteries-included. SQLite-shape schema migrated to
  Postgres with per-row XChaCha20-Poly1305, master key via systemd
  `LoadCredentialEncrypted=`, KMS scaffold (`LocalFile` + `VaultTransit` +
  `AwsKms`/`Pkcs11` stubs).
- **Audit** — append-only hash chain (Postgres pl/pgsql trigger computes
  prev-hash linkage on INSERT, no UPDATE/DELETE allowed). External anchor
  binary (`daimon-anchor snapshot/verify/list`) writes chain heads to
  `audit.anchors` + file mirror.
- **Multi-tenant isolation** — row-level security on every tenant-scoped
  table. Test suite proves cross-tenant credential reveal is blocked and
  per-tenant audit hash chains stay independent (`just test-isolation`).

## Architecture decisions (the anchored ones)

23 architectural decisions locked net — see
`git.wakbijok.uk/daimon/daimon-docs` (private) for the full spec. The
ones that shape the public API:

| ID | Decision | Why it matters |
| --- | --- | --- |
| D17 | Capability versioning (`(name, SemVer)`) | Worker agents and plans address each other by capability, not by process — version reqs in plans enforce compatibility |
| D18 | Compensating capabilities for saga rollback | Failed plan steps trigger their declared compensating step; full saga lands in 6.1 |
| D19 | Broker pattern — workers never see credentials | The Broker resolves `cred://X` and zeroizes after dispatch; workers receive a `SealedSession` only |
| D21 | Worker dependency restriction (`for-broker` feature) + cargo deny | Compile-time + CI enforcement that worker crates can't pull in vault/inventory directly |
| D22 | In-tree vault (supersedes the planned Vaultwarden integration) | Banking customers want one-vendor accountability for credentials |
| D23 | Append-only audit hash chain | Anchored at intervals via `daimon-anchor` for tamper-evidence |
| D25 | Production stack builder module | `daimon-app` assembles broker + vault + inventory + audit without violating D21 |

## Repos

| Repo | URL | Visibility |
| --- | --- | --- |
| Code | `https://github.com/wakbijok/daimon` + `git.wakbijok.uk/daimon/daimon` | Public (dual remote) |
| Docs (spec, runbooks, compliance) | `git.wakbijok.uk/daimon/daimon-docs` | Private |

## Crates

```
crates/
├── daimon-core            agent trait + capability registry + envelopes
├── daimon-runtime         in-proc bus + supervisor (restart-on-panic)
├── daimon-vault           in-tree credential vault (Postgres + XChaCha20)
├── daimon-inventory       target registry (Postgres + in-memory)
├── daimon-transport       Transport trait + russh impl + stubs
├── daimon-broker          PUBLIC action surface (D19 keystone)
├── daimon-audit           append-only hash-chained audit log
├── daimon-anchor          audit chain snapshot/verify CLI
├── daimon-db              refinery migrations + Pool (V001-V014)
├── daimon-kms             KMS abstraction (LocalFile / VaultTransit / KMS stubs)
├── daimon-memory          long-term memory tier (Postgres canonical)
├── daimon-rag             hybrid RAG (dense + sparse + rerank + packer)
├── daimon-redis           working memory tier (Redis + in-proc fallback)
├── daimon-llm             multi-provider client (Anthropic / OpenAI / Ollama)
├── daimon-guard           policy engine + kill switch + approval queue
├── daimon-orchestrator    plan persistence + DAG executor + LLM plans
├── daimon-observer        Prometheus ingest + named queries + anomalies
├── daimon-tool-network    first worker agent — RouterOS over SSH
├── daimon-tool-platform   Platform trait + PVE driver
├── daimon-pve             Proxmox VE REST client (preserved from pre-pivot)
├── daimon-app             Leptos 0.8 SSR + WASM hydrate (operator UI)
├── daimon-cli             daimon-demo + daimon-ingest + daimon-retrieve
└── daimon-migrate         SQLite → Postgres one-shot migrator
```

## Development

### One-time setup

```sh
brew install just postgresql@16 redis
cargo install cargo-leptos --locked
cargo install cargo-zigbuild   # musl linker on macOS, for `just build`
rustup target add wasm32-unknown-unknown
just qdrant-install            # downloads Qdrant native binary
```

### Daily flow

```sh
just pg-up && just pg-create-db && just pg-migrate    # Postgres
just qdrant-up                                         # Qdrant native bin
just redis-up                                          # Redis
just dev                                               # Leptos dev :3030
```

`just dev` overrides the workspace `bin-target-triple` (musl) with the host
triple so the binary runs on macOS. `just build` honours the musl config
and produces a static Linux x86_64 bin under
`target/x86_64-unknown-linux-musl/release/`.

### Recipes

| Command | What |
| --- | --- |
| `just` | List all recipes |
| `just dev` / `just dev-port 3030` | Dev server (Leptos, http://127.0.0.1:3030) |
| `just check` | SSR + hydrate compile check |
| `just test` | Full workspace tests |
| `just test-broker` | Broker keystone tests (D19/D21 + audit invariants) |
| `just test-isolation` | Multi-tenant isolation e2e (live Postgres) |
| `just test-rag` | Hybrid RAG e2e — first run downloads ~250 MB models |
| `just build` | Release musl bin for production deploy |
| `just keygen` | Generate `/tmp/daimon-dev.key` (auto-invoked by `just dev`) |
| `just dev-reset` | DESTRUCTIVE — wipe local vault / inventory / audit + master key |
| `just dev-reset-admin` | Re-seed admin/devadmin on next `just dev` |
| `just pg-up` / `pg-down` / `pg-status` / `pg-psql` | Postgres lifecycle |
| `just pg-migrate` / `pg-create-db` / `pg-drop-db` | DB schema lifecycle |
| `just pg-reset-tenant <slug>` | Wipe tenant data (keeps the tenant row) |
| `just migrate-data` / `migrate-data-verify` | One-shot SQLite → Postgres |
| `just qdrant-up` / `qdrant-down` / `qdrant-status` / `qdrant-reset` | Qdrant lifecycle |
| `just redis-up` / `redis-down` / `redis-status` | Redis lifecycle |
| `just audit-snapshot <tenant>` / `audit-verify` / `audit-anchors` | Audit chain ops |
| `just status` | Repo hygiene snapshot |

### Daemons

Native homelab dev — no Docker. Three databases run as native processes:

| Service | Port | Data | Why native |
| --- | --- | --- | --- |
| Postgres 16 | 5432 | `daimon` database, V001-V014 schemas | brew default, RLS works the same on macOS + Linux |
| Qdrant | 6333 REST / 6334 gRPC | `.qdrant-data/` | Pre-built binary, no JVM, faster boot than the OrbStack image |
| Redis 7 | 6379 | `~/.daimon/redis-data/` | Working memory tier — `conv_recent` + KV scratchpad + per-agent queue |

## Reading the codebase

Start with `crates/daimon-core` (agent trait + capability registry +
envelopes) and `crates/daimon-broker` (the D19 keystone — workers never see
raw credentials). The `Broker::execute` flow is the canonical action path:
inventory → vault → transport → audit. Tests in
`crates/daimon-broker/tests/agent_never_sees_credential.rs` prove the
invariant at compile time + runtime.

For the operator UI, `crates/daimon-app/src/app.rs` is the route table.
Server-fns live alongside their pages (`admin_*.rs`); WASM-side components
live in `components/`. The chat surface is a floating bubble mounted in
`components/layout.rs` so it survives route changes.

## License

MIT
