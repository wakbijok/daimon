# daimon systemd deployment

Single-organization, self-hosted deployment. daimon runs as one service
(`daimon.service`) — the in-process runtime (`InProcBus`) hosts the agents;
there is no per-agent process fan-out. (The multi-agent bus is wired up in a
later revival phase; a `daimon-nats.service` sidecar unit is provided for that
future path but is not required today.)

## Layout

| File | Purpose |
| --- | --- |
| `daimon.service` | The daimon application (web console + in-process runtime) |
| `install-daimon.sh` | One-shot installer — user, layout, PostgreSQL, master key, unit |
| `daimon-nats.service` | Optional NATS sidecar for the future multi-agent bus |

## Install (one-time)

Prerequisites on the box: PostgreSQL server running, and systemd with
`systemd-creds`.

```sh
# From a directory containing ./daimon-app, ./site/, and ./daimon.service:
sudo ./install-daimon.sh
```

The installer creates the `daimon` system user + `/var/lib/daimon`, provisions
the PostgreSQL `daimon` role + database, generates and encrypts the vault
master key (credential name `vault-master`), installs + starts the unit, and
verifies the service is actually active (it fails loudly instead of printing a
false success). Migrations run automatically on first boot. The first-boot
admin password is generated and logged — the installer surfaces it, or:

```sh
journalctl -u daimon.service | grep 'Generated admin password'
```

## Kill switch

Operator-only halt (per masterplan §2.4 / D13). daimon watches
`$DAIMON_DATA_DIR/KILL`; the unit sets `DAIMON_DATA_DIR=/var/lib/daimon`, so:

```sh
# Halt everything
sudo touch /var/lib/daimon/KILL

# Resume (no auto-resume by design)
sudo rm /var/lib/daimon/KILL
```

`SIGUSR1` to the daimon process is the redundant second path. (There is one
global kill switch — single-org has no per-tenant scope.)

## Credentials

The vault master key envelope lives at
`/etc/credstore.encrypted/daimon-vault-master`, encrypted with
`systemd-creds encrypt --name=vault-master`. systemd decrypts it on service
start and exposes it at `$CREDENTIALS_DIRECTORY/vault-master`, which
`daimon-vault`'s `MasterKey::from_systemd_credential()` reads. The credential
**name** must be `vault-master` to match that loader — the unit's
`LoadCredentialEncrypted=vault-master:...` line and the installer's
`--name=vault-master` both honor this.

For local development (no systemd), set `DAIMON_MASTER_KEY_FILE` to a 32-byte
key file instead; the loader logs a loud WARN in that mode.

## Self-update

> **Status: under rework (revival P5).** The release-artifact pipeline and the
> update hook's unit/path references are being reconciled; the self-update flow
> is not functional yet and this section will be rewritten when it lands. Until
> then, update by re-running `install-daimon.sh` with a new bundle.

Channel selection (`stable | beta`) is stored in `public.app_config` under
`update.channel`.
