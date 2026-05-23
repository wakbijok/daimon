# daimon systemd units (Phase 8)

Per-agent process packaging for production deployments. Pairs with the
in-process `InProcBus` dev model from Phases 0–7.

## Layout

| File | Purpose |
| --- | --- |
| `daimon-nats.service` | NATS sidecar — the inter-agent bus |
| `daimon-agent@.service` | Per-agent template — instance name = agent kind |

## Install (one-time)

```sh
sudo cp deploy/systemd/daimon-nats.service \
        deploy/systemd/daimon-agent@.service \
        /etc/systemd/system/
sudo systemctl daemon-reload
sudo useradd --system --no-create-home --shell /usr/sbin/nologin daimon
sudo install -d -o daimon -g daimon -m 0750 /var/lib/daimon
```

## Boot order

```sh
sudo systemctl enable --now daimon-nats.service
sudo systemctl enable --now daimon-agent@orchestrator.service
sudo systemctl enable --now daimon-agent@tool-network.service
sudo systemctl enable --now daimon-agent@tool-platform.service
sudo systemctl enable --now daimon-agent@observer.service
```

Each `daimon-agent@<kind>` loads the same binary with
`DAIMON_AGENT_KIND=<kind>` and connects to the local NATS sidecar.

## Kill switch

Operator-only halt (per masterplan §2.4 / D13):

```sh
# Halt everything for this LXC
sudo touch /var/lib/daimon/KILL

# Halt one tenant
sudo touch /var/lib/daimon/tenants/<tenant_id>/KILL

# Resume (no auto-resume by design)
sudo rm /var/lib/daimon/KILL
```

`SIGUSR1` to any daimon-agent process is the redundant second path.

## Credentials

The vault master key envelope lives at
`/etc/credstore.encrypted/daimon-vault-master`. systemd decrypts on
service start and exposes via `%d/daimon-vault-master`. The
`daimon-vault` crate reads from `$DAIMON_MASTER_KEY_FILE`.

Cloud / banking deploys swap the local-file envelope for an
HSM / Cloud KMS envelope (Phase 2c.1 carry-forward).

## Update mechanism (Phase 8 — `/admin/settings` → Update tab)

The Update tab in the operator UI writes a tag string to
`/var/lib/daimon/UPDATE_REQUESTED`. The `daimon-update.path` unit
watches that path and triggers `daimon-update.service` (oneshot), which
runs `daimon-update-hook.sh`. The hook:

1. Reads the target tag from the flag.
2. Downloads `daimon-app-x86_64-unknown-linux-musl.tar.gz` from the
   matching GitHub release.
3. Backs up the current binary to `/usr/local/lib/daimon/daimon-app.bak`.
4. Swaps in the new binary + site bundle.
5. Restarts `daimon-agent@*.service` + `daimon-app.service`.
6. If the app fails to start within 30s, restores the backup and exits
   with a failure code.

Install:

```sh
sudo cp deploy/systemd/daimon-update.path \
        deploy/systemd/daimon-update.service \
        /etc/systemd/system/
sudo install -m 0755 deploy/systemd/daimon-update-hook.sh \
        /etc/systemd/system/daimon-update-hook.sh
sudo systemctl daemon-reload
sudo systemctl enable --now daimon-update.path
```

Channel selection (`stable | beta | main`) is stored in
`public.app_config` under `update.channel`. The check button on the UI
queries the GitHub Releases API for the latest matching tag.
