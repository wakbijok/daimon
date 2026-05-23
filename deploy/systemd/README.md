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
