#!/usr/bin/env bash
# install-daimon.sh — run on the target VM as root, after scp'ing the bundle.
#
# Expects the following layout in the current directory:
#   ./daimon-app          (the linux-musl binary)
#   ./site/               (Leptos static assets directory)
#   ./daimon.service      (the systemd unit)
#
# Result:
#   /opt/daimon/daimon-app
#   /opt/daimon/site/...
#   /var/lib/daimon/      (DB lives here; owned by daimon:daimon)
#   /etc/systemd/system/daimon.service
#   daimon.service enabled + started, listening on 0.0.0.0:3000

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

# 4. Systemd unit
install -m 0644 -o root -g root ./daimon.service /etc/systemd/system/daimon.service
systemctl daemon-reload
systemctl enable daimon.service
systemctl restart daimon.service

# 5. Verify
sleep 2
echo
echo "== systemctl status =="
systemctl --no-pager status daimon.service | head -20
echo
echo "== listening ports =="
ss -ltnp | grep -E ':3000|daimon' || true
echo
echo "Open http://$(hostname -I | awk '{print $1}'):3000  (admin / admin)"
