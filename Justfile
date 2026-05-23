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
