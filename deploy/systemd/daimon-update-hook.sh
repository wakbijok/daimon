#!/usr/bin/env bash
# Phase 8 — daimon binary update hook.
#
# Called by daimon-update.service when the operator clicks Apply in
# /admin/settings → Update. Reads the target tag from
# /var/lib/daimon/UPDATE_REQUESTED, downloads the matching release asset
# from GitHub, swaps the binary, restarts the agent units, and rolls
# back on boot failure within 30s.
#
# This script is intentionally simple — no package manager assumed,
# minimal external deps (curl, tar, systemctl). The daimon binary is
# expected to live at /usr/local/bin/daimon-app on the LXC.

set -euo pipefail

FLAG=/var/lib/daimon/UPDATE_REQUESTED
TARGET_BIN=/usr/local/bin/daimon-app
BACKUP_BIN=/usr/local/lib/daimon/daimon-app.bak
STAGE_DIR=/var/lib/daimon/update-staging
SITE_TARGET=/var/lib/daimon/site
HOST_TRIPLE=x86_64-unknown-linux-musl

# Source endpoints follow the workflow locked 2026-05-23:
#   stable → GitHub releases (production-promoted)
#   beta   → GitLab releases (staging — default push target)
# Override via env on the systemd unit if your remotes differ.
GITHUB_REPO="${DAIMON_GITHUB_REPO:-wakbijok/daimon}"
GITLAB_HOST="${DAIMON_GITLAB_HOST:-git.wakbijok.uk}"
GITLAB_PROJECT="${DAIMON_GITLAB_PROJECT:-daimon/daimon}"

log() { printf '[update %s] %s\n' "$(date -Is)" "$*" >&2; }

if [ ! -f "$FLAG" ]; then
    log "no flag file present; nothing to do"
    exit 0
fi

# Flag format: two plaintext lines —
#   line 1: channel (stable | beta)
#   line 2: target tag (e.g. v1.0.0)
CHANNEL=$(sed -n 1p "$FLAG" | tr -d '[:space:]')
TAG=$(sed -n 2p "$FLAG" | tr -d '[:space:]')
if [ -z "$CHANNEL" ] || [ -z "$TAG" ]; then
    log "flag malformed — expected two lines (channel, tag); got '$(tr '\n' '|' < "$FLAG")'"
    exit 1
fi
log "applying update channel=$CHANNEL tag=$TAG"

mkdir -p "$STAGE_DIR" "$(dirname "$BACKUP_BIN")"
rm -rf "${STAGE_DIR:?}"/*

ASSET_NAME="daimon-app-${HOST_TRIPLE}.tar.gz"
case "$CHANNEL" in
    stable)
        DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download/${TAG}/${ASSET_NAME}"
        ;;
    beta)
        # GitLab releases serve assets via /-/releases/<tag>/downloads/<filename>
        ENC_PROJECT="${GITLAB_PROJECT//\//%2F}"
        DOWNLOAD_URL="https://${GITLAB_HOST}/${GITLAB_PROJECT}/-/releases/${TAG}/downloads/${ASSET_NAME}"
        _=$ENC_PROJECT
        ;;
    *)
        log "unknown channel '$CHANNEL' — refusing to apply"
        exit 1
        ;;
esac
log "downloading $DOWNLOAD_URL"
if ! curl -sSL -f -o "${STAGE_DIR}/${ASSET_NAME}" "$DOWNLOAD_URL"; then
    log "download failed; aborting"
    exit 1
fi

log "extracting"
tar -xzf "${STAGE_DIR}/${ASSET_NAME}" -C "$STAGE_DIR"

NEW_BIN=$(find "$STAGE_DIR" -type f -name daimon-app -perm -u+x | head -n1 || true)
if [ -z "$NEW_BIN" ] || [ ! -x "$NEW_BIN" ]; then
    log "no daimon-app binary in the asset; aborting"
    exit 1
fi
NEW_SITE=$(find "$STAGE_DIR" -type d -name site | head -n1 || true)

if [ -x "$TARGET_BIN" ]; then
    log "backing up current binary to $BACKUP_BIN"
    cp -p "$TARGET_BIN" "$BACKUP_BIN"
fi

log "installing new binary to $TARGET_BIN"
install -m 0755 "$NEW_BIN" "$TARGET_BIN"
if [ -n "$NEW_SITE" ] && [ -d "$NEW_SITE" ]; then
    log "installing new site bundle to $SITE_TARGET"
    rm -rf "$SITE_TARGET"
    mkdir -p "$SITE_TARGET"
    cp -r "$NEW_SITE/." "$SITE_TARGET/"
fi

log "restarting daimon services"
systemctl restart 'daimon-agent@*.service' daimon-app.service 2>/dev/null || true
sleep 30
if ! systemctl is-active --quiet daimon-app.service; then
    log "daimon-app failed to restart; rolling back"
    if [ -x "$BACKUP_BIN" ]; then
        install -m 0755 "$BACKUP_BIN" "$TARGET_BIN"
        systemctl restart daimon-app.service
        log "rollback complete"
    fi
    rm -f "$FLAG"
    exit 1
fi

log "update applied successfully to $TAG"
rm -f "$FLAG"
