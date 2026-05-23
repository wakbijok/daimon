# daimon

![status](https://img.shields.io/badge/status-in%20active%20development-orange)
![license](https://img.shields.io/badge/license-MIT-blue)
![rust](https://img.shields.io/badge/rust-2024%20edition-dea584)

Multi-agent system for managing infrastructure.

daimon is a Rust workspace that runs an orchestrator + worker agents over a
versioned capability bus, gated by a policy engine + kill switch + operator
approval inbox. It reaches managed infrastructure agentlessly over SSH,
REST, and SNMP. Memory is hybrid-RAG (dense + sparse + cross-encoder
rerank). Observability comes from a Prometheus ingestor that writes
normalised metrics + anomalies into long-term storage.

> **In active development.** APIs and storage shapes are still moving. Not
> ready for production use.

## What it does

- **Orchestrator** plans operator intents — LLM-emitted DAGs validated
  against the capability registry, executed topologically with
  `depends_on` barriers + replan-on-failure + operator-approval
  escalation on failure.
- **Worker agents** execute capabilities against real infrastructure
  over SSH / REST / SNMP. First two workers:
  - `tool-network` — read + (guarded) write on RouterOS over SSH with
    a per-capability allowlist
  - `tool-platform` — Proxmox VE driver implementing the generic
    `Platform` trait (snapshot/clone capability slots ready, write
    side is a future increment)
- **Guard** — TOML policy DSL with glob matching, in-process approval
  queue, and a kill switch (file watcher at `/var/lib/daimon/KILL` +
  `SIGUSR1`, manual `rm` to resume).
- **Memory + RAG** — relational canonical content tier + vector store
  with hybrid retrieval (dense + sparse with reciprocal-rank fusion)
  + cross-encoder rerank + greedy-MMR context packer with a token
  budget. A separate working-memory tier holds recent conversation
  turns, a KV scratchpad, and per-agent task queues.
- **Observer** — Prometheus ingest (PromQL instant + range), a
  TOML-defined named-query library, and an anomaly emitter that writes
  to both storage and the bus.
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

## Architectural choices worth knowing

| Choice | What it means |
| --- | --- |
| Broker pattern | Worker agents never see credentials. A broker resolves the credential reference, dispatches over a transport, and zeroizes after use. The keystone is enforced by a test that the worker crates cannot import the vault or inventory crates. |
| Capability versioning | Every action a worker exposes is a `(name, SemVer)` tuple. Plans reference capabilities by version requirement; the registry resolves to the actual provider. Agents can roll forward or back without breaking plans in flight. |
| Append-only audit hash chain | Every state-changing action emits an audit event. The DB computes the prev-hash linkage. Heads are anchored periodically for external tamper-evidence. |
| In-tree vault | Credentials live inside daimon. No external secrets manager required for the default deployment. KMS pluggable. |
| Operator kill switch | No agent can override it. Resume is manual; there's no auto-resume. |

## Crates

```
crates/
├── daimon-core            agent trait + capability registry + envelopes
├── daimon-runtime         in-proc bus + supervisor (restart-on-panic)
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
brew install just postgresql@16 redis
cargo install cargo-leptos --locked
cargo install cargo-zigbuild   # musl linker on macOS, for `just build`
rustup target add wasm32-unknown-unknown
just qdrant-install            # downloads Qdrant native binary
```

### Daily flow

```sh
just pg-up && just pg-create-db && just pg-migrate    # Postgres
just qdrant-up                                         # vector store
just redis-up                                          # working memory
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
| `just dev` / `just dev-port 3030` | Dev server (http://127.0.0.1:3030) |
| `just check` | SSR + hydrate compile check |
| `just test` | Full workspace tests |
| `just test-broker` | Broker keystone tests (credential boundary + audit invariants) |
| `just test-isolation` | Multi-tenant isolation e2e (live Postgres) |
| `just test-rag` | Hybrid RAG e2e — first run downloads ~250 MB models |
| `just build` | Release musl bin |
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

Native dev — no Docker. Three databases run as native processes:

| Service | Port | Data | Why native |
| --- | --- | --- | --- |
| Postgres 16 | 5432 | `daimon` database | brew default; row-level security works the same on macOS + Linux |
| Qdrant | 6333 REST / 6334 gRPC | `.qdrant-data/` | Pre-built binary, no JVM, fast boot |
| Redis 7 | 6379 | `~/.daimon/redis-data/` | Working memory tier — recent conversation, KV scratchpad, per-agent queues |

## Reading the codebase

Start with `crates/daimon-core` (agent trait + capability registry +
envelopes) and `crates/daimon-broker` (the credential-boundary keystone —
workers never see raw credentials). The `Broker::execute` flow is the
canonical action path: inventory → vault → transport → audit. Tests in
`crates/daimon-broker/tests/agent_never_sees_credential.rs` prove the
invariant.

For the operator UI, `crates/daimon-app/src/app.rs` is the route table.
Server-fns live alongside their pages (`admin_*.rs`); WASM-side components
live in `components/`. The chat surface is a floating bubble mounted in
`components/layout.rs` so it survives route changes.

## License

MIT
