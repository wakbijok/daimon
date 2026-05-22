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
# Opens at http://127.0.0.1:3000.
dev: keygen
    DAIMON_MASTER_KEY_FILE={{dev_key_file}} \
    DAIMON_DATA_DIR={{dev_data_dir}} \
    LEPTOS_BIN_TARGET_TRIPLE={{host_triple}} \
    cargo leptos serve

# Same as `dev` but on a custom port: `just dev-port 3030`.
dev-port port: keygen
    DAIMON_MASTER_KEY_FILE={{dev_key_file}} \
    DAIMON_DATA_DIR={{dev_data_dir}} \
    LEPTOS_BIN_TARGET_TRIPLE={{host_triple}} \
    LEPTOS_SITE_ADDR=127.0.0.1:{{port}} \
    cargo leptos serve

# DESTRUCTIVE: wipe local vault / inventory / audit + master key.
dev-reset:
    rm -rf {{dev_data_dir}} {{dev_key_file}}
    @echo "wiped {{dev_data_dir}} and {{dev_key_file}}"

# --- Check + Test ----------------------------------------------------------

# Fast compile check (ssr lib + hydrate WASM).
check:
    cargo check -p daimon-app --features ssr --no-default-features
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
