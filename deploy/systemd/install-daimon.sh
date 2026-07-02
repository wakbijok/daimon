#!/usr/bin/env bash
# install-daimon.sh — run on the target VM as root, after scp'ing the bundle.
#
# Prerequisites on the target box:
#   - PostgreSQL server installed and running (provides `sudo -u postgres psql`)
#   - systemd with systemd-creds (encrypted credential support)
#
# Expects the following layout in the current directory:
#   ./daimon-app          (the linux binary)
#   ./site/               (Leptos static assets directory)
#   ./daimon.service      (the systemd unit)
#
# Result:
#   /opt/daimon/daimon-app + /opt/daimon/site/...
#   /var/lib/daimon/                              (data dir; owned by daimon:daimon)
#   PostgreSQL role + database 'daimon'           (provisioned below)
#   /etc/credstore.encrypted/daimon-vault-master  (encrypted vault master key)
#   /etc/systemd/system/daimon.service
#   daimon.service enabled + started; migrations run on first boot;
#   listening on 0.0.0.0:3000.

set -euo pipefail

echo "== daimon installer =="

# 1. System user (no home, no shell — service account only)
if ! id daimon >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin daimon
    echo "  user 'daimon' created"
else
    echo "  user 'daimon' already exists"
fi

# 2. Layout
install -d -o root   -g root   -m 0755 /opt/daimon
install -d -o root   -g root   -m 0755 /opt/daimon/site
install -d -o daimon -g daimon -m 0750 /var/lib/daimon

# 3. Binary + assets
install -m 0755 -o root -g root ./daimon-app /opt/daimon/daimon-app
rm -rf /opt/daimon/site
cp -r ./site /opt/daimon/site
chown -R root:root /opt/daimon/site
find /opt/daimon/site -type d -exec chmod 0755 {} \;
find /opt/daimon/site -type f -exec chmod 0644 {} \;

# 4. PostgreSQL — provision the 'daimon' role + database (idempotent).
#    DAIMON_PG_URL=postgres:///daimon in the unit connects as the 'daimon'
#    OS user over the local socket (peer auth), so the PG role name matches.
if ! command -v psql >/dev/null 2>&1; then
    echo "ERROR: PostgreSQL client not found. Install postgresql first." >&2
    exit 1
fi
echo "== provisioning PostgreSQL role + database 'daimon' =="
sudo -u postgres psql -v ON_ERROR_STOP=1 -tAc \
    "SELECT 1 FROM pg_roles WHERE rolname='daimon'" | grep -q 1 \
    || sudo -u postgres psql -v ON_ERROR_STOP=1 -c "CREATE ROLE daimon LOGIN;"
sudo -u postgres psql -v ON_ERROR_STOP=1 -tAc \
    "SELECT 1 FROM pg_database WHERE datname='daimon'" | grep -q 1 \
    || sudo -u postgres createdb -O daimon daimon
echo "  role + database ready (migrations run on first daimon boot)"

# 5. Vault master key — generate once, encrypt with systemd-creds. The
#    credential NAME must be 'vault-master' to match the unit's
#    LoadCredentialEncrypted= and daimon-vault's loader.
install -d -o root -g root -m 0700 /etc/credstore.encrypted
if [ ! -f /etc/credstore.encrypted/daimon-vault-master ]; then
    echo "== generating + encrypting vault master key =="
    umask 077
    keytmp="$(mktemp)"
    head -c 32 /dev/urandom > "$keytmp"
    systemd-creds encrypt --name=vault-master "$keytmp" \
        /etc/credstore.encrypted/daimon-vault-master
    shred -u "$keytmp" 2>/dev/null || rm -f "$keytmp"
    chmod 0600 /etc/credstore.encrypted/daimon-vault-master
    echo "  master key envelope written"
else
    echo "  master key envelope already present — leaving as-is"
fi

# 6. Systemd unit
install -m 0644 -o root -g root ./daimon.service /etc/systemd/system/daimon.service
systemctl daemon-reload
systemctl enable daimon.service
systemctl restart daimon.service

# 7. Verify — fail loudly if the unit is crash-looping instead of printing
#    a false success.
echo
echo "== verifying =="
sleep 3
if ! systemctl is-active --quiet daimon.service; then
    echo "ERROR: daimon.service is not active (likely crash-looping)." >&2
    echo "Recent logs:" >&2
    journalctl -u daimon.service -n 40 --no-pager >&2 || true
    exit 1
fi
echo "  daimon.service is active"
echo
echo "== listening ports =="
ss -ltnp | grep -E ':3000|daimon' || true
echo
# Surface the generated first-boot admin password (only logged when no
# DAIMON_ADMIN_PASSWORD was set — the default).
pw_line="$(journalctl -u daimon.service --no-pager 2>/dev/null | grep -m1 'Generated admin password' || true)"
ip="$(hostname -I 2>/dev/null | awk '{print $1}')"
echo "Open http://${ip:-<host>}:3000"
if [ -n "$pw_line" ]; then
    echo "First-boot admin login: username 'admin', ${pw_line#*: } (from the journal)"
else
    echo "Admin login: username 'admin'. If you set DAIMON_ADMIN_PASSWORD, use that;"
    echo "otherwise check: journalctl -u daimon.service | grep 'Generated admin password'"
fi
