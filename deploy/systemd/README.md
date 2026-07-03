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

## Messaging gateways (Telegram / Matrix)

daimon can be reached from a chat platform (P4). A channel message runs the
**same** authenticated chat + tool path a browser turn takes — bound to a real
IAM identity, gated by the same policy + approval. Configure channels under
**Settings → Channels** (`admin`-gated). Nothing is enabled by default; with no
channel enabled, the `POST /api/v1/gw/{channel}` route 404s and no poller runs.

**Secrets live in the vault, referenced by credential name** (never in
`app_config` or a log — FR-GW-17). Create each bot secret as an `ApiToken`
credential under **Settings → Vault & KMS**, then name it in the Channels tab.

### Telegram (inbound webhook)

Needs a public HTTPS ingress (a reverse proxy terminating TLS in front of
`:3000`). Telegram POSTs updates to `https://<host>/api/v1/gw/telegram`,
authenticated by a secret token daimon verifies constant-time.

1. Create the bot via `@BotFather`; note the bot token.
2. Store both as vault `ApiToken` credentials, e.g. `gw-telegram-bot-token` and
   `gw-telegram-webhook-secret` (any random 32+ char string for the latter).
3. In **Channels**, set:
   `channels.telegram.enabled = true`,
   `channels.telegram.bot_token_cred = gw-telegram-bot-token`,
   `channels.telegram.webhook_secret_cred = gw-telegram-webhook-secret`.
4. Register the webhook with Telegram (once), pinning the secret token:
   ```sh
   curl "https://api.telegram.org/bot<TOKEN>/setWebhook" \
     -d "url=https://<host>/api/v1/gw/telegram" \
     -d "secret_token=<the webhook secret>"
   ```
5. Restart daimon. Enrol the operator's Telegram **numeric user id** →
   daimon username under **Channels → Identity enrolment**. An unmapped handle
   is refused fail-closed.

### Matrix (`/sync` poller)

No public ingress needed — daimon long-polls the homeserver as a bot.

1. Create a bot account on your homeserver; obtain a long-lived **access token**.
2. Store it as a vault `ApiToken` credential, e.g. `gw-matrix-access-token`.
3. In **Channels**, set:
   `channels.matrix.enabled = true`,
   `channels.matrix.homeserver = https://matrix.example.org`,
   `channels.matrix.access_token_cred = gw-matrix-access-token`.
4. Restart daimon. Invite the bot to a room; enrol the operator's **MXID**
   (`@user:server`) → daimon username under **Identity enrolment**. The bot skips
   its own messages and resumes from a persisted `/sync` cursor across restarts.

> Outbound alert routing (anomaly / approval push to a channel) is a later-phase
> deliverable — P4 is inbound + reply only.

## Self-update

> **Status: under rework (revival P5).** The release-artifact pipeline and the
> update hook's unit/path references are being reconciled; the self-update flow
> is not functional yet and this section will be rewritten when it lands. Until
> then, update by re-running `install-daimon.sh` with a new bundle.

Channel selection (`stable | beta`) is stored in `public.app_config` under
`update.channel`.
