# Justfile for daimon — local dev + prod build wrappers.
# Run `just` to list recipes.

set shell := ["bash", "-c"]

# Host triple (auto-detected). Overrides the workspace's musl bin-target so
# `just dev` produces a binary cargo-leptos can actually execute locally.
host_triple := `rustc -vV | grep '^host:' | cut -d' ' -f2`

# Dev master key — created on demand by `just keygen`. /tmp scope; never commit.
dev_key_file := "/tmp/daimon-dev.key"

# Local data dir (vault.db, inventory.db, audit.db, known_hosts).
dev_data_dir := "./daimon-data"

# Dev admin login. Used only on first seed of daimon.db's users table.
# Production deploy gets a real password via systemd env; this default is
# dev-only convenience. Reset existing seed with `just dev-reset-admin`.
dev_admin_password := "devadmin"

# Default: list recipes.
default:
    @just --list

# --- Dev -------------------------------------------------------------------

# Generate dev master key (idempotent — skip if file already exists).
keygen:
    @if [ ! -f {{dev_key_file}} ]; then \
        head -c 32 /dev/urandom > {{dev_key_file}} && \
        chmod 600 {{dev_key_file}} && \
        echo "generated dev master key at {{dev_key_file}}"; \
    else \
        echo "dev master key already exists at {{dev_key_file}}"; \
    fi

# Local dev server. Native host bin + WASM hot reload via cargo-leptos.
# Opens at http://127.0.0.1:3030 (workspace default 3000 collides with the
# Hermes WhatsApp bridge on Wak's laptop; production deploy still uses 3000
# per workspace site-addr).
dev: keygen
    DAIMON_MASTER_KEY_FILE={{dev_key_file}} \
    DAIMON_DATA_DIR={{dev_data_dir}} \
    DAIMON_ADMIN_PASSWORD={{dev_admin_password}} \
    LEPTOS_BIN_TARGET_TRIPLE={{host_triple}} \
    LEPTOS_SITE_ADDR=127.0.0.1:3030 \
    cargo leptos serve

# Same as `dev` but on a custom port: `just dev-port 3030`.
dev-port port: keygen
    DAIMON_MASTER_KEY_FILE={{dev_key_file}} \
    DAIMON_DATA_DIR={{dev_data_dir}} \
    DAIMON_ADMIN_PASSWORD={{dev_admin_password}} \
    LEPTOS_BIN_TARGET_TRIPLE={{host_triple}} \
    LEPTOS_SITE_ADDR=127.0.0.1:{{port}} \
    cargo leptos serve

# DESTRUCTIVE: wipe local vault / inventory / audit + master key.
dev-reset:
    rm -rf {{dev_data_dir}} {{dev_key_file}}
    @echo "wiped {{dev_data_dir}} and {{dev_key_file}}"

# Drop the admin user row so the next `just dev` re-seeds with
# {{dev_admin_password}}. Preserves PVE cluster registry + everything else
# in daimon.db.
dev-reset-admin:
    @if [ -f daimon.db ]; then \
        sqlite3 daimon.db "DELETE FROM users WHERE username='admin';" && \
        echo "deleted admin user; next 'just dev' will re-seed with password: {{dev_admin_password}}"; \
    else \
        echo "no daimon.db found; nothing to reset"; \
    fi

# --- Qdrant (vector tier — Phase 3) ----------------------------------------
#
# Dev runs the native Qdrant binary (no Docker, no container runtime overhead).
# Binary lives at ~/.daimon/bin/qdrant; data lives at ./.qdrant-data; pid + log
# at ./.qdrant-data/qdrant.{pid,log}. Production uses the cluster build per
# MASTERPLAN §3.2 — same protocol (gRPC :6334), same qdrant-client code path.

qdrant_version := "v1.18.1"
qdrant_bin_dir := env_var('HOME') + "/.daimon/bin"
qdrant_bin := qdrant_bin_dir + "/qdrant"
qdrant_data := "./.qdrant-data"
qdrant_pid := qdrant_data + "/qdrant.pid"
qdrant_log := qdrant_data + "/qdrant.log"

# Download the Qdrant binary if not already present (idempotent).
qdrant-install:
    @if [ ! -x {{qdrant_bin}} ]; then \
        mkdir -p {{qdrant_bin_dir}} && \
        echo "downloading qdrant {{qdrant_version}} for arm64-darwin..." && \
        curl -sSL -o /tmp/qdrant.tar.gz \
            "https://github.com/qdrant/qdrant/releases/download/{{qdrant_version}}/qdrant-aarch64-apple-darwin.tar.gz" && \
        tar -xzf /tmp/qdrant.tar.gz -C {{qdrant_bin_dir}} && \
        chmod +x {{qdrant_bin}} && \
        rm /tmp/qdrant.tar.gz && \
        echo "installed at {{qdrant_bin}}"; \
    else \
        echo "qdrant already installed at {{qdrant_bin}}"; \
    fi

# Start Qdrant in the background. REST :6333, gRPC :6334, dashboard at http://localhost:6333/dashboard.
qdrant-up: qdrant-install
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p {{qdrant_data}}
    if [ -f {{qdrant_pid}} ] && kill -0 "$(cat {{qdrant_pid}})" 2>/dev/null; then
        echo "qdrant already running (pid $(cat {{qdrant_pid}}))"
        exit 0
    fi
    cd {{qdrant_data}}
    QDRANT__STORAGE__STORAGE_PATH=./storage \
    QDRANT__STORAGE__SNAPSHOTS_PATH=./snapshots \
    QDRANT__SERVICE__HTTP_PORT=6333 \
    QDRANT__SERVICE__GRPC_PORT=6334 \
    nohup {{qdrant_bin}} </dev/null >qdrant.log 2>&1 &
    echo $! > qdrant.pid
    disown
    echo "qdrant started, pid $(cat qdrant.pid), REST :6333, gRPC :6334, dashboard http://localhost:6333/dashboard"

# Stop Qdrant (preserves data).
qdrant-down:
    @if [ -f {{qdrant_pid}} ]; then \
        kill $(cat {{qdrant_pid}}) 2>/dev/null && \
        rm -f {{qdrant_pid}} && \
        echo "stopped qdrant"; \
    else \
        echo "not running"; \
    fi

# DESTRUCTIVE: stop qdrant AND wipe local data.
qdrant-reset: qdrant-down
    rm -rf {{qdrant_data}}
    @echo "wiped {{qdrant_data}}"

# Show qdrant status + a curl healthcheck.
qdrant-status:
    @if [ -f {{qdrant_pid}} ] && kill -0 $(cat {{qdrant_pid}}) 2>/dev/null; then \
        echo "qdrant running, pid $(cat {{qdrant_pid}})"; \
    else \
        echo "qdrant not running"; \
    fi
    @curl -sS http://localhost:6333/healthz 2>&1 || echo "(REST endpoint not reachable yet)"

# --- Postgres (relational tier — Phase 2c) ---------------------------------
#
# Dev uses the brew-installed Postgres 16 with brew's default data directory.
# Managed via pg_ctl (not brew services) so the daemon is daimon-scoped and
# doesn't autostart at login. Same client code path (sqlx) for prod cluster.

pg_bin := "/opt/homebrew/opt/postgresql@16/bin"
pg_data := "/opt/homebrew/var/postgresql@16"
pg_log := pg_data + "/server.log"
pg_port := "5432"
pg_user := env_var('USER')
pg_db := "daimon"
pg_url := "postgres://" + pg_user + "@localhost:" + pg_port + "/" + pg_db

# Start Postgres in the background.
pg-up:
    #!/usr/bin/env bash
    set -euo pipefail
    if {{pg_bin}}/pg_ctl -D {{pg_data}} status >/dev/null 2>&1; then
        echo "postgres already running"
    else
        {{pg_bin}}/pg_ctl -D {{pg_data}} -l {{pg_log}} start
        echo "postgres started, port {{pg_port}}, data {{pg_data}}, log {{pg_log}}"
    fi

# Stop Postgres.
pg-down:
    @{{pg_bin}}/pg_ctl -D {{pg_data}} stop 2>/dev/null && echo "stopped postgres" || echo "not running"

# Show status + database list.
pg-status:
    @{{pg_bin}}/pg_ctl -D {{pg_data}} status 2>&1 || true
    @{{pg_bin}}/psql -p {{pg_port}} -d postgres -c '\l' 2>&1 | head -10 || echo "(no client connection)"

# Create the daimon database (idempotent).
pg-create-db:
    @{{pg_bin}}/psql -p {{pg_port}} -d postgres -tc "SELECT 1 FROM pg_database WHERE datname='{{pg_db}}'" | grep -q 1 && \
        echo "database {{pg_db}} already exists" || \
        ({{pg_bin}}/createdb -p {{pg_port}} {{pg_db}} && echo "created database {{pg_db}}")

# DESTRUCTIVE: drop the daimon database (dev only).
pg-drop-db:
    @{{pg_bin}}/dropdb -p {{pg_port}} --if-exists {{pg_db}}
    @echo "dropped database {{pg_db}}"

# Interactive psql shell to the daimon database.
pg-psql:
    @{{pg_bin}}/psql -p {{pg_port}} -d {{pg_db}}

# Run pending migrations against the daimon database.
pg-migrate:
    DAIMON_PG_URL="{{pg_url}}" cargo run -p daimon-db --bin daimon-migrate

# Echo the dev Postgres URL (for sourcing into env: `export DAIMON_PG_URL=$(just pg-url)`).
pg-url:
    @echo "{{pg_url}}"

# DESTRUCTIVE: wipe one tenant's data without affecting other tenants. Preserves
# the public.tenants row itself; clears vault/inventory/audit/clusters/plans/users.
pg-reset-tenant slug:
    #!/usr/bin/env bash
    set -euo pipefail
    {{pg_bin}}/psql -p {{pg_port}} -d {{pg_db}} -v ON_ERROR_STOP=1 <<SQL
    DO \$\$ DECLARE t_id UUID;
    BEGIN
        SELECT id INTO t_id FROM public.tenants WHERE slug = '{{slug}}';
        IF t_id IS NULL THEN
            RAISE EXCEPTION 'no tenant with slug = {{slug}}';
        END IF;
        DELETE FROM vault.credentials WHERE tenant_id = t_id;
        DELETE FROM inventory.targets WHERE tenant_id = t_id;
        DELETE FROM public.clusters WHERE tenant_id = t_id;
        DELETE FROM public.plan_steps WHERE plan_id IN (SELECT id FROM public.plans WHERE tenant_id = t_id);
        DELETE FROM public.plans WHERE tenant_id = t_id;
        DELETE FROM public.role_grants WHERE user_id IN (SELECT id FROM public.users WHERE tenant_id = t_id);
        DELETE FROM public.users WHERE tenant_id = t_id;
    END \$\$;
    SQL
    echo "tenant {{slug}} content wiped"

# --- Phase 2c data migration + anchoring ---------------------------------

# SQLite → Postgres data migrate (one-shot, idempotent). Run after pg-migrate.
migrate-data:
    DAIMON_PG_URL="{{pg_url}}" cargo run -p daimon-migrate-data --bin daimon-migrate-data -- run

# Side-by-side row count compare (sqlite vs pg).
migrate-data-verify:
    DAIMON_PG_URL="{{pg_url}}" cargo run -p daimon-migrate-data --bin daimon-migrate-data -- verify

# Snapshot the current audit chain head for a tenant (defaults to `default`).
audit-snapshot tenant="default":
    DAIMON_PG_URL="{{pg_url}}" cargo run -p daimon-anchor --bin daimon-anchor -- snapshot --tenant {{tenant}}

# Verify a tenant's audit chain by recomputing hashes from canonical fields.
audit-verify tenant="default":
    DAIMON_PG_URL="{{pg_url}}" cargo run -p daimon-anchor --bin daimon-anchor -- verify --tenant {{tenant}}

# List anchors for a tenant.
audit-anchors tenant="default":
    DAIMON_PG_URL="{{pg_url}}" cargo run -p daimon-anchor --bin daimon-anchor -- list --tenant {{tenant}}

# Placeholder: DEK rotation. Phase 2c.1 wires this to daimon-vault.
vault-rotate-dek:
    @echo "vault-rotate-dek is a Phase 2c.1 deliverable — daimon-kms crate exists but the rotate orchestration is not yet wired."
    @echo "See daimon-docs/plans/2026-05-23-phase-2c-compliance-posture-plan.md D4."
    @exit 1

# Multi-tenant isolation e2e test. Requires Postgres running.
test-isolation:
    DAIMON_PG_URL="{{pg_url}}" cargo test -p daimon-broker --test multi_tenant_isolation -- --ignored

# Phase 3 hybrid RAG e2e test (Postgres + Qdrant + fastembed models).
# First run downloads ~250 MB of model weights into ~/.cache/fastembed/.
test-rag:
    DAIMON_PG_URL="{{pg_url}}" cargo test -p daimon-rag --test phase3_e2e -- --ignored

# --- Check + Test ----------------------------------------------------------

# Fast compile check (ssr lib + hydrate WASM). Keep default features ON —
# leptos's transitive deps trip when `--no-default-features` strips
# `leptos_config` from its activated set during host SSR check.
check:
    cargo check -p daimon-app --features ssr
    cargo check -p daimon-app --features hydrate --no-default-features --target wasm32-unknown-unknown

# All workspace tests.
test:
    cargo test --workspace

# Broker-only tests (keystone D19/D21 + audit invariants).
test-broker:
    cargo test -p daimon-broker

# --- Production build -----------------------------------------------------

# Release build via workspace config (musl target → static Linux x86_64 bin
# at target/x86_64-unknown-linux-musl/release/daimon-app + bundled site at
# target/site/). Requires `rustup target add x86_64-unknown-linux-musl` and
# a musl-compatible linker (cargo-zigbuild or homebrew musl-cross).
build:
    cargo leptos build --release

# Repo hygiene snapshot.
status:
    git status
    git diff --stat

# --- Release promotion ---------------------------------------------------
#
# Three-stage workflow:
#   local working tree → `git push` → staging (GitLab)  → `just promote` → production (GitHub)
#
# `just promote` is the explicit staging → production push. It checks the
# working tree is clean, fetches staging, refuses unless local main matches
# staging/main (no untracked staging-side commits to bypass), and fast-forwards
# main onto production. Use this instead of raw `git push production` so the
# preconditions are always checked.
#
# Refuses if the tree is dirty, if local main is not in sync with staging/main,
# or if production has diverged from local.

# Promote staging HEAD to production (GitHub). See block comment above.
promote:
    @set -e; \
    if [ -n "$(git status --porcelain)" ]; then \
        echo "refuse: working tree is dirty — commit or stash before promoting"; \
        exit 1; \
    fi; \
    branch=$(git rev-parse --abbrev-ref HEAD); \
    if [ "$branch" != "main" ]; then \
        echo "refuse: on branch '$branch'; promote only from main"; \
        exit 1; \
    fi; \
    echo "→ fetching staging + production"; \
    git fetch staging; \
    git fetch production; \
    local_head=$(git rev-parse main); \
    staging_head=$(git rev-parse staging/main); \
    if [ "$local_head" != "$staging_head" ]; then \
        echo "refuse: local main ($local_head) != staging/main ($staging_head)"; \
        echo "        run 'git push' first to align staging with local"; \
        exit 1; \
    fi; \
    production_head=$(git rev-parse production/main 2>/dev/null || echo "none"); \
    if [ "$production_head" = "$local_head" ]; then \
        echo "already in sync: production/main = $local_head"; \
        exit 0; \
    fi; \
    if [ "$production_head" != "none" ]; then \
        if ! git merge-base --is-ancestor "$production_head" "$local_head"; then \
            echo "refuse: production/main ($production_head) is NOT an ancestor of local main"; \
            echo "        production has diverged — investigate before force-pushing"; \
            exit 1; \
        fi; \
    fi; \
    echo "→ promoting $local_head to production/main"; \
    git push production main; \
    echo "✓ promoted to production"

# --- Redis (hot working memory tier — Phase 4) -----------------------------
#
# Dev uses native brew Redis with a daimon-scoped data directory. Same
# pattern as Postgres + Qdrant — no Docker, native binary.

redis_bin := "/opt/homebrew/opt/redis/bin"
redis_data := "./.redis-data"
redis_log := redis_data + "/redis.log"
redis_pid := redis_data + "/redis.pid"
redis_port := "6379"
redis_url := "redis://localhost:" + redis_port

# Start Redis in the background.
redis-up:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p {{redis_data}}
    if [ -f {{redis_pid}} ] && kill -0 "$(cat {{redis_pid}})" 2>/dev/null; then
        echo "redis already running, pid $(cat {{redis_pid}})"
    else
        {{redis_bin}}/redis-server \
            --daemonize yes \
            --dir $(pwd)/{{redis_data}} \
            --pidfile $(pwd)/{{redis_pid}} \
            --logfile $(pwd)/{{redis_log}} \
            --port {{redis_port}} \
            --appendonly yes
        echo "redis started, port {{redis_port}}, data {{redis_data}}, log {{redis_log}}"
    fi

# Stop Redis.
redis-down:
    @if [ -f {{redis_pid}} ] && kill -0 "$(cat {{redis_pid}})" 2>/dev/null; then \
        kill "$(cat {{redis_pid}})" && rm -f {{redis_pid}} && echo "stopped redis"; \
    else \
        echo "redis not running"; \
    fi

# Status + ping.
redis-status:
    @if [ -f {{redis_pid}} ] && kill -0 "$(cat {{redis_pid}})" 2>/dev/null; then \
        echo "redis running, pid $(cat {{redis_pid}})"; \
    else \
        echo "redis not running"; \
    fi
    @{{redis_bin}}/redis-cli -p {{redis_port}} ping 2>&1 || echo "(PING unreachable)"

# Echo the dev Redis URL.
redis-url:
    @echo "{{redis_url}}"

# --- VictoriaMetrics (time-series tier — Phase 8 lock) -------------------
#
# Dev uses native brew VictoriaMetrics single-binary with a daimon-scoped
# data directory. Same pattern as Postgres + Qdrant + Redis — no Docker,
# native binary. Prod uses the cluster trio (vmstorage + vminsert + vmselect).
#
# VM speaks PromQL (and MetricsQL superset) — `daimon-observer::PrometheusClient`
# treats VM as a drop-in Prometheus endpoint. Default HTTP port 8428 is the
# VM single-node convention; `/api/v1/write` accepts Prometheus remote_write
# (snappy+protobuf) for ingest.

vm_bin := "/opt/homebrew/opt/victoriametrics/bin"
vm_data := "./.victoria-metrics-data"
vm_log := vm_data + "/victoria-metrics.log"
vm_pid := vm_data + "/victoria-metrics.pid"
vm_port := "8428"
vm_retention := "12" # months; banking floor needs 7y via downsampling in prod
vm_url := "http://localhost:" + vm_port

# Start VictoriaMetrics in the background. REST :8428.
vm-up:
    @mkdir -p {{vm_data}}
    @if [ -f {{vm_pid}} ] && kill -0 "$(cat {{vm_pid}})" 2>/dev/null; then \
        echo "vm already running, pid $(cat {{vm_pid}})"; \
        exit 0; \
    fi
    @rm -f {{vm_pid}}
    @if [ ! -x {{vm_bin}}/victoria-metrics ]; then \
        echo "victoria-metrics binary not found — run: brew install victoriametrics"; \
        exit 1; \
    fi
    @{{vm_bin}}/victoria-metrics \
        -httpListenAddr=127.0.0.1:{{vm_port}} \
        -storageDataPath={{vm_data}} \
        -retentionPeriod={{vm_retention}} \
        >> {{vm_log}} 2>&1 & \
        echo $! > {{vm_pid}}
    @sleep 1
    @if kill -0 "$(cat {{vm_pid}})" 2>/dev/null; then \
        echo "vm started, pid $(cat {{vm_pid}}), :{{vm_port}}"; \
    else \
        echo "vm failed to start, see {{vm_log}}"; \
        exit 1; \
    fi

# Stop VictoriaMetrics.
vm-down:
    @if [ -f {{vm_pid}} ] && kill -0 "$(cat {{vm_pid}})" 2>/dev/null; then \
        kill "$(cat {{vm_pid}})" && rm -f {{vm_pid}} && echo "stopped vm"; \
    else \
        echo "vm not running"; \
    fi

# Status + healthcheck.
vm-status:
    @if [ -f {{vm_pid}} ] && kill -0 "$(cat {{vm_pid}})" 2>/dev/null; then \
        echo "vm running, pid $(cat {{vm_pid}})"; \
    else \
        echo "vm not running"; \
    fi
    @curl -sf {{vm_url}}/health 2>&1 || echo "(health unreachable)"

# Echo the dev VictoriaMetrics URL (use as DAIMON_VM_URL).
vm-url:
    @echo "{{vm_url}}"

# DESTRUCTIVE: stop vm AND wipe local data.
vm-reset:
    @just vm-down
    @rm -rf {{vm_data}}
    @echo "wiped {{vm_data}}"

# --- NornicDB (graph tier — Phase 8 lock) --------------------------------
#
# Dev uses native NornicDB lite. Manual install one-time:
#   1. Download NornicDB-1.1.0-arm64-lite.pkg from
#      https://github.com/orneryd/NornicDB/releases/latest
#   2. Open the .pkg and run the installer
#   3. Confirm `nornicdb --version` resolves
#
# NornicDB speaks Neo4j Bolt protocol on :7687 (Cypher queries) and HTTP
# on :7474. Wire-compat with Qdrant gRPC on :6334 too (not used here —
# vector tier stays on Qdrant proper for Phase 8; NornicDB-as-vector is a
# Phase 9 unification candidate).

nornicdb_data := "./.nornicdb-data"
nornicdb_log := nornicdb_data + "/nornicdb.log"
nornicdb_pid := nornicdb_data + "/nornicdb.pid"
nornicdb_http_port := "7474"
nornicdb_bolt_port := "7687"
nornicdb_url := "bolt://localhost:" + nornicdb_bolt_port

# Print install instructions (NornicDB ships as a macOS .pkg, no brew yet).
nornicdb-install:
    @echo "NornicDB install (one-time, manual):"
    @echo "  1. Download:  https://github.com/orneryd/NornicDB/releases/latest"
    @echo "                pick NornicDB-1.1.0-arm64-lite.pkg (or -full.pkg for Metal GPU)"
    @echo "  2. Open the .pkg and run the installer (requires admin password)"
    @echo "  3. Verify:    nornicdb --version"
    @echo ""
    @echo "Linux/prod path: build from source via 'go build -o nornicdb ./cmd/nornicdb'"
    @echo "in a clone of github.com/orneryd/NornicDB. Native NornicDB Linux binaries"
    @echo "are not yet published — track the releases page for changes."

# Start NornicDB in the background. Bolt :7687, HTTP :7474.
nornicdb-up:
    @mkdir -p {{nornicdb_data}}
    @if [ -f {{nornicdb_pid}} ] && kill -0 "$(cat {{nornicdb_pid}})" 2>/dev/null; then \
        echo "nornicdb already running, pid $(cat {{nornicdb_pid}})"; \
        exit 0; \
    fi
    @rm -f {{nornicdb_pid}}
    @if ! command -v nornicdb >/dev/null 2>&1; then \
        echo "nornicdb binary not on PATH — run: just nornicdb-install"; \
        exit 1; \
    fi
    @nornicdb serve \
        --data-dir {{nornicdb_data}} \
        --bolt-port {{nornicdb_bolt_port}} \
        --http-port {{nornicdb_http_port}} \
        >> {{nornicdb_log}} 2>&1 & \
        echo $! > {{nornicdb_pid}}
    @sleep 1
    @if kill -0 "$(cat {{nornicdb_pid}})" 2>/dev/null; then \
        echo "nornicdb started, pid $(cat {{nornicdb_pid}}), bolt :{{nornicdb_bolt_port}}, http :{{nornicdb_http_port}}"; \
    else \
        echo "nornicdb failed to start, see {{nornicdb_log}}"; \
        exit 1; \
    fi

# Stop NornicDB.
nornicdb-down:
    @if [ -f {{nornicdb_pid}} ] && kill -0 "$(cat {{nornicdb_pid}})" 2>/dev/null; then \
        kill "$(cat {{nornicdb_pid}})" && rm -f {{nornicdb_pid}} && echo "stopped nornicdb"; \
    else \
        echo "nornicdb not running"; \
    fi

# Status + ping.
nornicdb-status:
    @if [ -f {{nornicdb_pid}} ] && kill -0 "$(cat {{nornicdb_pid}})" 2>/dev/null; then \
        echo "nornicdb running, pid $(cat {{nornicdb_pid}})"; \
    else \
        echo "nornicdb not running"; \
    fi
    @curl -sf http://localhost:{{nornicdb_http_port}}/ 2>&1 | head -3 || echo "(HTTP unreachable)"

# Echo the dev NornicDB URL (use as DAIMON_GRAPH_URL).
nornicdb-url:
    @echo "{{nornicdb_url}}"

# DESTRUCTIVE: stop nornicdb AND wipe local data.
nornicdb-reset:
    @just nornicdb-down
    @rm -rf {{nornicdb_data}}
    @echo "wiped {{nornicdb_data}}"
