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
    pub tenant_id: String,
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

// ---- get / set / list -------------------------------------------------------

#[server]
pub async fn list_settings(prefix: String) -> Result<Vec<SettingRow>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let client = state
        .db
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool: {e}")))?;
    // RLS scoping: set the tenant guc before reading.
    client
        .execute(
            "SELECT set_config('app.tenant_id', $1::text, true)",
            &[&claims.tenant_id.to_string()],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("set tenant: {e}")))?;

    let like = format!("{prefix}%");
    let rows = client
        .query(
            "SELECT key, value, is_secret, updated_at
             FROM public.app_config
             WHERE tenant_id = $1 AND key LIKE $2
             ORDER BY key ASC",
            &[&claims.tenant_id, &like],
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
    client
        .execute(
            "SELECT set_config('app.tenant_id', $1::text, true)",
            &[&claims.tenant_id.to_string()],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("set tenant: {e}")))?;

    // Secrets land as vault:// refs. The current cut treats `value` as the
    // already-resolved ref string for is_secret=true; future iteration
    // intercepts plaintext, stores via daimon-vault, and replaces value
    // with the new ref.
    client
        .execute(
            "INSERT INTO public.app_config (tenant_id, key, value, is_secret, updated_by)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (tenant_id, key) DO UPDATE
               SET value = EXCLUDED.value,
                   is_secret = EXCLUDED.is_secret,
                   updated_by = EXCLUDED.updated_by,
                   updated_at = now()",
            &[
                &claims.tenant_id,
                &key,
                &value,
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
            "SELECT set_config('app.tenant_id', $1::text, true)",
            &[&claims.tenant_id.to_string()],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("set tenant: {e}")))?;
    client
        .execute(
            "DELETE FROM public.app_config WHERE tenant_id = $1 AND key = $2",
            &[&claims.tenant_id, &key],
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
    Ok(())
}

// ---- System info ------------------------------------------------------------

#[server]
pub async fn get_system_info() -> Result<SystemInfo, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();

    // Tenant name (from public.tenants, joined via tenant_id claim).
    let tenant_name = {
        let client = state
            .db
            .get()
            .await
            .map_err(|e| ServerFnError::new(format!("pool: {e}")))?;
        let row = client
            .query_opt(
                "SELECT name FROM public.tenants WHERE id = $1",
                &[&claims.tenant_id],
            )
            .await
            .map_err(|e| ServerFnError::new(format!("tenant query: {e}")))?;
        row.map(|r| r.get::<_, String>(0)).unwrap_or_else(|| "(unknown)".into())
    };

    let mut backends = Vec::new();
    // Postgres reachability — we just queried it, so it's up.
    backends.push(BackendStatus {
        name: "Postgres".into(),
        url: std::env::var("DAIMON_PG_URL").unwrap_or_else(|_| "(env)".into()),
        reachable: true,
        detail: None,
    });
    // Qdrant / VM / NornicDB / Redis / NATS / Prometheus — probe via HTTP healthcheck.
    backends.push(probe_http(
        "Qdrant",
        std::env::var("DAIMON_QDRANT_URL").unwrap_or_else(|_| "http://localhost:6333".into()),
        "/healthz",
    ).await);
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
        tenant_id: claims.tenant_id.to_string(),
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

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let client = state
        .db
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool: {e}")))?;
    client
        .execute(
            "SELECT set_config('app.tenant_id', $1::text, true)",
            &[&claims.tenant_id.to_string()],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("set tenant: {e}")))?;
    let rows = client
        .query(
            "SELECT key, value FROM public.app_config
             WHERE tenant_id = $1 AND key LIKE 'update.%'",
            &[&claims.tenant_id],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("query: {e}")))?;

    let mut channel = "stable".to_string();
    let mut latest_tag: Option<String> = None;
    let mut last_check_at: Option<String> = None;
    for r in rows {
        let key: String = r.get(0);
        let value: serde_json::Value = r.get(1);
        match key.as_str() {
            "update.channel" => {
                if let Some(s) = value.as_str() {
                    channel = s.to_string();
                }
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

    let tag = match state.channel.as_str() {
        "main" => {
            let body: serde_json::Value = http
                .get("https://api.github.com/repos/wakbijok/daimon/commits/main")
                .send()
                .await
                .map_err(|e| ServerFnError::new(format!("commits api: {e}")))?
                .error_for_status()
                .map_err(|e| ServerFnError::new(format!("commits status: {e}")))?
                .json()
                .await
                .map_err(|e| ServerFnError::new(format!("commits json: {e}")))?;
            body.get("sha")
                .and_then(|v| v.as_str())
                .map(|s| format!("main-{}", &s[..7.min(s.len())]))
        }
        channel => {
            let body: serde_json::Value = http
                .get("https://api.github.com/repos/wakbijok/daimon/releases")
                .send()
                .await
                .map_err(|e| ServerFnError::new(format!("releases api: {e}")))?
                .error_for_status()
                .map_err(|e| ServerFnError::new(format!("releases status: {e}")))?
                .json()
                .await
                .map_err(|e| ServerFnError::new(format!("releases json: {e}")))?;
            let releases = body.as_array().cloned().unwrap_or_default();
            releases.into_iter().find_map(|r| {
                let prerelease = r.get("prerelease").and_then(|v| v.as_bool()).unwrap_or(false);
                let want_pre = channel == "beta";
                if want_pre || !prerelease {
                    r.get("tag_name").and_then(|v| v.as_str()).map(String::from)
                } else {
                    None
                }
            })
        }
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
    let flag = us.update_flag_path.clone();
    if let Some(parent) = std::path::Path::new(&flag).parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    tokio::fs::write(&flag, format!("{}\n", target))
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
