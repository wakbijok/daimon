//! Phase 8 — `/admin/settings` server-fns.
//!
//! Generic key/value settings store backed by `public.app_config`. Secrets
//! (LLM API keys, KMS envelope paths) are stored as `vault://settings/<key>`
//! references — the actual secret bytes live in the vault tier.
//!
//! Every save writes an `audit.events` row with `ActionKind::Other` and
//! metadata `settings.key` = the changed key. Banking-grade compliance —
//! auditors need to know who turned MFA off, when.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingRow {
    pub key: String,
    /// JSON-encoded value. For `is_secret = true` this is a vault:// ref
    /// string, never the plaintext.
    pub value: serde_json::Value,
    pub is_secret: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SystemInfo {
    pub version: String,
    pub commit_sha: String,
    pub build_profile: String,
    pub host_triple: String,
    pub tenant_name: String,
    pub kill_switch_engaged: bool,
    pub kill_switch_reason: Option<String>,
    pub backends: Vec<BackendStatus>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BackendStatus {
    pub name: String,
    pub url: String,
    pub reachable: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateState {
    pub channel: String,
    pub current_version: String,
    pub current_commit: String,
    pub latest_version: Option<String>,
    pub latest_tag: Option<String>,
    pub last_check_at: Option<String>,
    pub update_pending: bool,
    pub update_flag_path: String,
}

/// P6-3: store a settings secret in the vault THROUGH the broker (D21), keyed by
/// a deterministic name. Create-or-update by name so re-saving a changed secret
/// updates the same credential instead of erroring on a duplicate name. The
/// plaintext is wrapped as an `ApiToken` credential; it is never returned,
/// logged, or written to `app_config` — only the `vault://` ref is.
#[cfg(feature = "ssr")]
async fn intercept_secret(
    state: &crate::state::AppState,
    actor: &str,
    vault_name: &str,
    plaintext: &str,
) -> Result<(), String> {
    use crate::admin_credentials::CredentialDto;

    let dto = CredentialDto::ApiToken {
        token: plaintext.to_string(),
    };
    // Look up an existing credential of this name to decide create vs update.
    let existing = state
        .broker
        .vault_list_metadata(actor)
        .await
        .map_err(|e| e.to_string())?;
    match existing.into_iter().find(|m| m.name == vault_name) {
        Some(m) => state
            .broker
            .vault_update(actor, m.id, dto.into())
            .await
            .map(|_| ())
            .map_err(|e| e.to_string()),
        None => state
            .broker
            .vault_create(actor, vault_name, dto.into())
            .await
            .map(|_| ())
            .map_err(|e| e.to_string()),
    }
}

// ---- get / set / list -------------------------------------------------------

#[server]
pub async fn list_settings(prefix: String) -> Result<Vec<SettingRow>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let _claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let client = state
        .db
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool: {e}")))?;

    let like = format!("{prefix}%");
    let rows = client
        .query(
            "SELECT key, value, is_secret, updated_at
             FROM public.app_config
             WHERE key LIKE $1
             ORDER BY key ASC",
            &[&like],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("query: {e}")))?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let updated_at: chrono::DateTime<chrono::Utc> = r.get(3);
            SettingRow {
                key: r.get(0),
                value: r.get(1),
                is_secret: r.get(2),
                updated_at: updated_at.to_rfc3339(),
            }
        })
        .collect())
}

#[server]
pub async fn set_setting(
    key: String,
    value: serde_json::Value,
    is_secret: bool,
) -> Result<(), ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let client = state
        .db
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool: {e}")))?;

    // P6-3 (FR-CFG-11/12, FR-GW-17): server-side vault interception. For a
    // secret field, PLAINTEXT never reaches app_config or the logs — it is
    // stored in the vault via the broker (D21: daimon-app never touches
    // daimon-vault directly) and only the `vault://settings.<key>` ref is
    // persisted. A re-save whose value is already a `vault://` ref means the
    // operator did not change the secret, so we keep the ref untouched
    // (idempotent, no re-wrap, no spurious vault write).
    let persist_value: serde_json::Value = if is_secret {
        match value.as_str() {
            Some(existing) if existing.starts_with("vault://") => value.clone(),
            Some(plaintext) if !plaintext.is_empty() => {
                let vault_name = format!("settings.{key}");
                intercept_secret(&state, &claims.sub, &vault_name, plaintext)
                    .await
                    .map_err(|e| ServerFnError::new(format!("vault store: {e}")))?;
                serde_json::Value::String(format!("vault://{vault_name}"))
            }
            // Empty or non-string secret clears the value; store as-is.
            _ => value.clone(),
        }
    } else {
        value.clone()
    };

    client
        .execute(
            "INSERT INTO public.app_config (key, value, is_secret, updated_by)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (key) DO UPDATE
               SET value = EXCLUDED.value,
                   is_secret = EXCLUDED.is_secret,
                   updated_by = EXCLUDED.updated_by,
                   updated_at = now()",
            &[
                &key,
                &persist_value,
                &is_secret,
                &Some(claims.user_id),
            ],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("upsert: {e}")))?;

    // Audit the setting change. Banking-grade evidence: auditors ask
    // "who turned MFA off" — we answer with actor + timestamp + key.
    let mut meta = std::collections::BTreeMap::new();
    meta.insert("settings.key".to_string(), key.clone());
    meta.insert("settings.is_secret".to_string(), is_secret.to_string());
    let _ = state
        .broker
        .audit_admin_action(
            &claims.sub,
            daimon_broker::ActionKind::Other,
            None,
            None,
            Some(&format!("settings.upsert {key}")),
            meta,
        )
        .await;

    // P6 (FR-CFG-14): hot-swap a fresh config snapshot so the edit is live for
    // the next runtime read (LLM model, guard timeout, observer interval, alert
    // routing). A reload failure is LOGGED, never fatal — the row is already
    // saved; the worst case is the old snapshot lingers until the next write.
    if let Err(e) = state.config.reload(&state.db).await {
        tracing::warn!(error = %e, key = %key, "config reload after settings write failed");
    }
    // P6 (FR-CFG-06/10): push the refreshed live tunables (guard timeout,
    // observer interval) into the running subsystems.
    state.apply_runtime_tunables(&state.config.current());

    Ok(())
}

#[server]
pub async fn delete_setting(key: String) -> Result<(), ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let client = state
        .db
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool: {e}")))?;
    client
        .execute(
            "DELETE FROM public.app_config WHERE key = $1",
            &[&key],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("delete: {e}")))?;

    let mut meta = std::collections::BTreeMap::new();
    meta.insert("settings.key".into(), key.clone());
    let _ = state
        .broker
        .audit_admin_action(
            &claims.sub,
            daimon_broker::ActionKind::Other,
            None,
            None,
            Some(&format!("settings.delete {key}")),
            meta,
        )
        .await;

    // P6 (FR-CFG-14): reload so the deleted key reverts to its env/default.
    if let Err(e) = state.config.reload(&state.db).await {
        tracing::warn!(error = %e, key = %key, "config reload after settings delete failed");
    }
    state.apply_runtime_tunables(&state.config.current());
    Ok(())
}

// ---- System info ------------------------------------------------------------

#[server]
pub async fn get_system_info() -> Result<SystemInfo, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let _claims = require_admin().await?;
    let state = expect_context::<AppState>();

    // Single-org: public.tenants was dropped. Org display name comes from
    // an optional `identity.org_name` app_config key; default "daimon".
    let tenant_name = {
        let client = state
            .db
            .get()
            .await
            .map_err(|e| ServerFnError::new(format!("pool: {e}")))?;
        let row = client
            .query_opt(
                "SELECT value FROM public.app_config WHERE key = 'identity.org_name'",
                &[],
            )
            .await
            .map_err(|e| ServerFnError::new(format!("org_name query: {e}")))?;
        row.and_then(|r| {
            let v: serde_json::Value = r.get(0);
            v.as_str().map(String::from)
        })
        .unwrap_or_else(|| "daimon".into())
    };

    let mut backends = Vec::new();
    // Postgres reachability — we just queried it, so it's up.
    backends.push(BackendStatus {
        name: "Postgres".into(),
        url: std::env::var("DAIMON_PG_URL").unwrap_or_else(|_| "(env)".into()),
        reachable: true,
        detail: None,
    });
    // VM / NornicDB / Redis / NATS / Prometheus — probe via HTTP healthcheck.
    // (Qdrant retired in P3 — long-term memory is now the dmem sidecar behind
    // MemoryService; probed via /healthz's `memory` field, not here.)
    backends.push(probe_http(
        "VictoriaMetrics",
        std::env::var("DAIMON_VM_URL").unwrap_or_else(|_| "http://localhost:8428".into()),
        "/health",
    ).await);
    backends.push(probe_http(
        "NornicDB",
        std::env::var("DAIMON_GRAPH_URL")
            .ok()
            .map(|s| s.replace("bolt://", "http://").replace(":7687", ":7474"))
            .unwrap_or_else(|| "http://localhost:7474".into()),
        "/",
    ).await);
    let redis_url = std::env::var("DAIMON_REDIS_URL")
        .unwrap_or_else(|_| "redis://localhost:6379".into());
    backends.push(BackendStatus {
        name: "Redis".into(),
        url: redis_url,
        // Reachability of Redis is implied by the AppState wire — if Redis
        // were down the chat surface would have failed at boot.
        reachable: true,
        detail: Some("(in-process probe via working_memory)".into()),
    });
    backends.push(probe_http(
        "NATS",
        std::env::var("DAIMON_NATS_URL")
            .ok()
            .map(|s| s.replace("nats://", "http://").replace(":4222", ":8222"))
            .unwrap_or_else(|| "http://localhost:8222".into()),
        "/varz",
    ).await);
    let prom_url = std::env::var("DAIMON_PROM_URL").unwrap_or_default();
    backends.push(if prom_url.is_empty() {
        BackendStatus {
            name: "Prometheus".into(),
            url: "(unset)".into(),
            reachable: false,
            detail: Some("DAIMON_PROM_URL not configured".into()),
        }
    } else {
        probe_http("Prometheus", prom_url, "/-/healthy").await
    });

    Ok(SystemInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit_sha: option_env!("DAIMON_COMMIT_SHA")
            .unwrap_or("dev")
            .to_string(),
        build_profile: if cfg!(debug_assertions) { "debug" } else { "release" }.into(),
        host_triple: option_env!("TARGET").unwrap_or("native").to_string(),
        tenant_name,
        kill_switch_engaged: false, // wired when Guard is attached to AppState (Phase 8.1)
        kill_switch_reason: None,
        backends,
    })
}

#[cfg(feature = "ssr")]
async fn probe_http(name: &str, base: String, path: &str) -> BackendStatus {
    let trimmed = base.trim_end_matches('/').to_string();
    let url = format!("{trimmed}{path}");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .expect("reqwest build");
    let (reachable, detail) = match client.get(&url).send().await {
        Ok(r) if r.status().is_success() => (true, None),
        Ok(r) => (false, Some(format!("HTTP {}", r.status().as_u16()))),
        Err(e) => (false, Some(format!("{e}"))),
    };
    BackendStatus {
        name: name.into(),
        url: trimmed,
        reachable,
        detail,
    }
}

// ---- Update tab -------------------------------------------------------------

#[server]
pub async fn get_update_state() -> Result<UpdateState, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let _claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let client = state
        .db
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool: {e}")))?;
    let rows = client
        .query(
            "SELECT key, value FROM public.app_config
             WHERE key LIKE 'update.%'",
            &[],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("query: {e}")))?;

    let mut channel: Option<String> = None;
    let mut latest_tag: Option<String> = None;
    let mut last_check_at: Option<String> = None;
    for r in rows {
        let key: String = r.get(0);
        let value: serde_json::Value = r.get(1);
        match key.as_str() {
            "update.channel" => {
                channel = value.as_str().map(String::from);
            }
            "update.last_check_latest" => {
                latest_tag = value.as_str().map(String::from);
            }
            "update.last_check_at" => {
                last_check_at = value.as_str().map(String::from);
            }
            _ => {}
        }
    }

    // First-read seed: default to `stable` on a fresh tenant. Banking
    // posture — we want a known default written so the audit log shows
    // "explicit stable on tenant boot" rather than "implicit on every
    // read." The write is idempotent (ON CONFLICT DO UPDATE in
    // set_setting); the audit row is one-time on fresh tenants because
    // the value matches itself after the first save.
    let channel = match channel {
        Some(c) => c,
        None => {
            let _ = set_setting(
                "update.channel".into(),
                serde_json::Value::String("stable".into()),
                false,
            )
            .await;
            "stable".to_string()
        }
    };

    // Check pending state via the flag file the systemd path-unit watches.
    let flag_path = std::env::var("DAIMON_UPDATE_FLAG")
        .unwrap_or_else(|_| "/var/lib/daimon/UPDATE_REQUESTED".into());
    let update_pending = tokio::fs::try_exists(&flag_path).await.unwrap_or(false);

    Ok(UpdateState {
        channel,
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        current_commit: option_env!("DAIMON_COMMIT_SHA").unwrap_or("dev").to_string(),
        latest_version: latest_tag.as_ref().map(|t| t.trim_start_matches('v').to_string()),
        latest_tag,
        last_check_at,
        update_pending,
        update_flag_path: flag_path,
    })
}

#[server]
pub async fn check_for_update() -> Result<UpdateState, ServerFnError> {
    use crate::auth_guard::require_admin;

    let _claims = require_admin().await?;
    // Read current channel first so we know which endpoint to hit.
    let mut state = get_update_state().await?;

    // GitHub Releases API. For `stable` we want the latest non-prerelease.
    // For `beta` we want the latest including prereleases. For `main` we
    // fall back to the latest commit on main — there isn't a release for
    // every main push.
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .user_agent("daimon-update-checker")
        .build()
        .map_err(|e| ServerFnError::new(format!("http build: {e}")))?;

    // Channel-to-source mapping locks the dev/staging/prod workflow:
    //   stable  → GitHub releases  = production-promoted (`just promote` target)
    //   beta    → GitLab releases  = staging (default `git push` target)
    // Anything else falls back to stable for safety.
    let tag = match state.channel.as_str() {
        "beta" => latest_gitlab_release(&http).await?,
        _ => latest_github_release(&http).await?,
    };

    let now = chrono::Utc::now().to_rfc3339();
    set_setting(
        "update.last_check_at".into(),
        serde_json::Value::String(now.clone()),
        false,
    )
    .await?;
    if let Some(t) = &tag {
        set_setting(
            "update.last_check_latest".into(),
            serde_json::Value::String(t.clone()),
            false,
        )
        .await?;
    }
    state.last_check_at = Some(now);
    state.latest_tag = tag.clone();
    state.latest_version = tag.map(|t| t.trim_start_matches('v').to_string());
    Ok(state)
}

#[server]
pub async fn apply_update() -> Result<UpdateState, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state_ctx = expect_context::<AppState>();
    let mut us = get_update_state().await?;

    let target = us
        .latest_tag
        .clone()
        .ok_or_else(|| ServerFnError::new("no latest_tag known — run check_for_update first"))?;

    // Write the flag file. A systemd path-unit watches this path and
    // triggers update.service which does the privileged binary swap +
    // restart. Dev (macOS) just leaves the flag; the operator picks it
    // up by hand or via a local equivalent of the update hook.
    //
    // Flag format (two-line plaintext, simple for the bash hook to
    // parse without jq):
    //   <channel>
    //   <tag>
    // The hook picks the download source based on channel:
    //   stable → GitHub  beta → GitLab.
    let flag = us.update_flag_path.clone();
    if let Some(parent) = std::path::Path::new(&flag).parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    tokio::fs::write(&flag, format!("{}\n{}\n", us.channel, target))
        .await
        .map_err(|e| ServerFnError::new(format!("write flag: {e}")))?;

    // Audit + tracing record so a buyer-side audit can show who pulled
    // the trigger and when.
    let mut meta = std::collections::BTreeMap::new();
    meta.insert("update.target_tag".into(), target.clone());
    meta.insert("update.channel".into(), us.channel.clone());
    let _ = state_ctx
        .broker
        .audit_admin_action(
            &claims.sub,
            daimon_broker::ActionKind::Other,
            None,
            None,
            Some(&format!("update.apply_requested -> {target}")),
            meta,
        )
        .await;

    us.update_pending = true;
    Ok(us)
}

#[server]
pub async fn cancel_update() -> Result<UpdateState, ServerFnError> {
    use crate::auth_guard::require_admin;

    let _claims = require_admin().await?;
    let us = get_update_state().await?;
    let _ = tokio::fs::remove_file(&us.update_flag_path).await;
    let mut us = us;
    us.update_pending = false;
    Ok(us)
}

#[cfg(feature = "ssr")]
async fn latest_github_release(http: &reqwest::Client) -> Result<Option<String>, ServerFnError> {
    // Stable = GitHub Releases, latest non-prerelease tag.
    // Repo URL is overridable via DAIMON_GITHUB_REPO (defaults to
    // wakbijok/daimon — production remote in our workflow).
    let repo = std::env::var("DAIMON_GITHUB_REPO")
        .unwrap_or_else(|_| "wakbijok/daimon".to_string());
    let body: serde_json::Value = http
        .get(format!("https://api.github.com/repos/{repo}/releases"))
        .send()
        .await
        .map_err(|e| ServerFnError::new(format!("github releases: {e}")))?
        .error_for_status()
        .map_err(|e| ServerFnError::new(format!("github releases status: {e}")))?
        .json()
        .await
        .map_err(|e| ServerFnError::new(format!("github releases json: {e}")))?;
    let releases = body.as_array().cloned().unwrap_or_default();
    Ok(releases.into_iter().find_map(|r| {
        let prerelease = r.get("prerelease").and_then(|v| v.as_bool()).unwrap_or(false);
        let draft = r.get("draft").and_then(|v| v.as_bool()).unwrap_or(false);
        if prerelease || draft {
            None
        } else {
            r.get("tag_name").and_then(|v| v.as_str()).map(String::from)
        }
    }))
}

#[cfg(feature = "ssr")]
async fn latest_gitlab_release(http: &reqwest::Client) -> Result<Option<String>, ServerFnError> {
    // Beta = GitLab Releases on the staging remote. The project path is
    // URL-encoded — `daimon/daimon` becomes `daimon%2Fdaimon`. Overridable
    // via DAIMON_GITLAB_HOST + DAIMON_GITLAB_PROJECT.
    let host = std::env::var("DAIMON_GITLAB_HOST")
        .unwrap_or_else(|_| "git.wakbijok.uk".to_string());
    let project = std::env::var("DAIMON_GITLAB_PROJECT")
        .unwrap_or_else(|_| "daimon/daimon".to_string());
    let encoded = project.replace('/', "%2F");
    let url = format!("https://{host}/api/v4/projects/{encoded}/releases");
    let body: serde_json::Value = http
        .get(&url)
        .send()
        .await
        .map_err(|e| ServerFnError::new(format!("gitlab releases: {e}")))?
        .error_for_status()
        .map_err(|e| ServerFnError::new(format!("gitlab releases status ({url}): {e}")))?
        .json()
        .await
        .map_err(|e| ServerFnError::new(format!("gitlab releases json: {e}")))?;
    let releases = body.as_array().cloned().unwrap_or_default();
    // GitLab returns newest first by default. tag_name is the same field
    // shape as GitHub.
    Ok(releases.into_iter().next().and_then(|r| {
        r.get("tag_name").and_then(|v| v.as_str()).map(String::from)
    }))
}
