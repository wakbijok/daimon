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
