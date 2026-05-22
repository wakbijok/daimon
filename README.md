# daimon

AI-driven system engineer for Proxmox.

daimon is a daemon that monitors, manages, and automates Proxmox VE infrastructure with built-in intelligence senses (monitoring), hands (SSH + API), and a brain (AIOps).

## Status

Early development. Not ready for use.

## Building

```sh
cargo build --release
```

## Development

`Justfile` wraps the common workflows. One-time setup:

```sh
brew install just
cargo install cargo-leptos --locked
cargo install cargo-zigbuild  # for `just build` musl linker on macOS
```

Recipes:

| Command | What it does |
| --- | --- |
| `just` | List all recipes |
| `just dev` | Local dev server (native host bin, hot reload, http://127.0.0.1:3030) |
| `just dev-port 3030` | Same on a custom port |
| `just keygen` | Generate `/tmp/daimon-dev.key` (auto-invoked by `just dev`) |
| `just dev-reset` | DESTRUCTIVE — wipe local vault / inventory / audit + master key |
| `just dev-reset-admin` | Drop the admin row in `daimon.db`; next `just dev` re-seeds with `devadmin` |
| `just check` | SSR + hydrate compile check |
| `just test` | Workspace tests |
| `just test-broker` | Broker-only tests (D19/D21 + audit keystones) |
| `just build` | Release musl bin for production deploy |
| `just status` | `git status` + diffstat |

`just dev` overrides the workspace `bin-target-triple` (musl) with the host
triple so the binary actually runs on macOS; `just build` honours the workspace
config and produces the static Linux x86_64 bin under
`target/x86_64-unknown-linux-musl/release/`.

## License

MIT
