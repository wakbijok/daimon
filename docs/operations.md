# daimon operations guide

Operator runbook for a single-organization, self-hosted daimon deployment.
Companion to [`deploy/systemd/README.md`](../deploy/systemd/README.md) (install
mechanics) and [`config-reference.md`](config-reference.md) (every config key,
code-derived).

## 1. Install / first boot

See [`deploy/systemd/README.md`](../deploy/systemd/README.md). In short, from a
directory containing `./daimon-app`, `./site/`, and `./daimon.service`:

```sh
sudo ./install-daimon.sh
```

The installer creates the `daimon` user + `/var/lib/daimon`, provisions the
PostgreSQL role + database, generates and encrypts the vault master key,
installs and starts the unit, and verifies it is active. Migrations run
automatically on first boot. The first-boot admin password is generated and
logged — retrieve it with:

```sh
journalctl -u daimon.service | grep 'Generated admin password'
```

There is **no static default credential** — the password is random per install
(AC-P0-05). Log in, then change it and create per-operator IAM users
(Settings → IAM).

## 2. Kill switch

Operator-only halt (D13). daimon watches `$DAIMON_DATA_DIR/KILL`; the unit sets
`DAIMON_DATA_DIR=/var/lib/daimon`:

```sh
sudo touch /var/lib/daimon/KILL   # halt every write immediately
sudo rm    /var/lib/daimon/KILL   # resume (no auto-resume by design)
```

## 3. Updates

The `daimon-update.path` unit watches an update flag; `Settings → Update` drives
the channel/check/apply. Apply swaps the binary + site bundle and restarts the
unit; migrations run forward on boot. Roll back by restoring the previous binary
+ a Postgres dump from before the update (see §4).

## 4. Backup & restore

Two things must be backed up together:

1. **Postgres** — the entire `daimon` database (vault ciphertext, inventory,
   audit chain, plans, IAM, chat history). Standard `pg_dump`:
   ```sh
   sudo -u postgres pg_dump -Fc daimon > daimon-$(date +%F).dump
   ```
2. **The vault master key** — see the escrow runbook below. The Postgres dump
   holds only the *encrypted* credential blobs; without the master key they are
   unrecoverable.

Restore Postgres:

```sh
sudo systemctl stop daimon.service
sudo -u postgres pg_restore -c -d daimon daimon-YYYY-MM-DD.dump
```

## 5. Master-key escrow + restore runbook (NFR-DATA-02) — READ CAREFULLY

The vault master key encrypts every stored credential. On the running host it
lives at `/etc/credstore.encrypted/daimon-vault-master` as a **systemd-creds
envelope**, and systemd decrypts it to `$CREDENTIALS_DIRECTORY/vault-master` at
start (the unit's `LoadCredentialEncrypted=vault-master:...`).

> **The envelope is HOST-BOUND.** `systemd-creds encrypt` seals to the host
> (TPM2 and/or the host key). The encrypted file **cannot be decrypted on a
> different machine**. So a host loss without an off-box escrow of the
> *plaintext* key = **permanent loss of every vault secret**. Backing up the
> encrypted envelope alone is NOT enough.

### Escrow (do this ONCE, at provision time)

When the master key is first generated (32 random bytes), export a copy of the
**plaintext** key to a sealed, off-box store (a password manager entry, an HSM,
or offline media) BEFORE it is only present as a host-bound envelope:

```sh
# Generate + escrow, then encrypt to the host envelope:
head -c 32 /dev/urandom | base64 > /root/vault-master.plaintext   # transient
#  → copy the contents of /root/vault-master.plaintext into your sealed escrow
sudo systemd-creds encrypt --name=vault-master \
     /root/vault-master.plaintext /etc/credstore.encrypted/daimon-vault-master
shred -u /root/vault-master.plaintext                             # wipe transient
```

Store the escrowed plaintext with the same care as the vault it protects.

### Restore (on a NEW host)

1. Provision the box (install PostgreSQL, the `daimon` user + dirs).
2. Retrieve the escrowed **plaintext** master key into a transient file
   `vault-master.plaintext`.
3. Re-seal it to the NEW host and install the envelope:
   ```sh
   sudo systemd-creds encrypt --name=vault-master \
        vault-master.plaintext /etc/credstore.encrypted/daimon-vault-master
   shred -u vault-master.plaintext
   ```
4. Restore the Postgres dump (§4).
5. Start the unit: `sudo systemctl start daimon.service`.

The credential name **must** be `vault-master` (the loader looks for exactly
that). A restore with the wrong key or a missing escrow leaves every credential
undecryptable.

## 6. Build & release constraints (musl-static + zvec-off)

The production binary is a **fully static musl** build
(`target/x86_64-unknown-linux-musl/release/daimon-app`, `just build`), so it
drops onto any Linux host with no glibc/runtime dependency.

Long-term memory is the **dmem HTTP sidecar** (`daimon-memory` behind the
`MemoryService` trait), NOT an embedded vector index — the native zvec `.so`
cannot be statically linked into a musl binary (a LOCKED P3 decision). The 1.0
release ships keyword-only recall (SDS §6.9 option-A) when the sidecar is
unconfigured; the dense-vector packaging spike is post-1.0. Set
`DAIMON_DMEM_URL` + the `dmem-bearer` vault credential to enable recall.

## 7. Configuration reference

Every environment variable and `app_config` key the binary reads, with defaults,
is in [`config-reference.md`](config-reference.md) — **generated from the code**
and drift-checked in CI (a stale reference fails the build). Resolution
precedence is DB `app_config` → env var → compiled default; bootstrap secrets
(`DAIMON_PG_URL`, the master key, `DAIMON_DATA_DIR`) stay env/credential-sourced
and are never in `app_config`.
