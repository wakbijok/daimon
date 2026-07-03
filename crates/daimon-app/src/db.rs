//! Postgres-backed app data layer (Phase 2c D3b).
//!
//! Async helpers against `daimon_db::Pool`. Replaces the prior rusqlite-on-
//! tokio-Mutex implementation. Single-org: tenant scoping is gone — the DB
//! migrations dropped `tenant_id` columns and `public.tenants`, and
//! `app_config` is now a key-only store.

#[cfg(feature = "ssr")]
use anyhow::{Context, Result};
#[cfg(feature = "ssr")]
use chrono::{DateTime, Utc};
#[cfg(feature = "ssr")]
use daimon_db::Pool;
#[cfg(feature = "ssr")]
use uuid::Uuid;

// ---- bootstrap --------------------------------------------------------------

/// Initialise the Postgres pool + run migrations. Returns a cheaply-cloneable
/// pool handle.
#[cfg(feature = "ssr")]
pub async fn init_pool(pg_url: &str) -> Result<Pool> {
    daimon_db::run_migrations(pg_url)
        .await
        .context("run migrations")?;
    let pool = daimon_db::build_pool(pg_url).context("build pool")?;
    Ok(pool)
}

// ---- users ------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[derive(Debug, Clone)]
pub struct UserRow {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    /// Account lifecycle status: 'active' | 'disabled' | 'locked'. Login
    /// rejects any non-'active' account (FR-IAM-07).
    pub status: String,
    pub roles: Vec<String>,
}

#[cfg(feature = "ssr")]
pub async fn find_user(pool: &Pool, username: &str) -> Result<Option<UserRow>> {
    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_opt(
            "SELECT u.id, u.username, u.password_hash, u.status,
                    COALESCE(
                        ARRAY(
                            SELECT r.slug
                            FROM public.role_grants rg
                            JOIN public.roles r ON r.id = rg.role_id
                            WHERE rg.user_id = u.id
                            ORDER BY r.is_system DESC, r.slug
                        ),
                        ARRAY[]::TEXT[]
                    ) AS roles
             FROM public.users u
             WHERE u.username = $1
             LIMIT 1",
            &[&username],
        )
        .await
        .context("find_user")?;
    Ok(row.map(|r| UserRow {
        id: r.get(0),
        username: r.get(1),
        password_hash: r.get(2),
        status: r.get(3),
        roles: r.get(4),
    }))
}

/// Create a user with an explicit role set (single-org; no tenant scope).
/// Each slug is granted via an INSERT..SELECT so an unknown slug is silently
/// skipped rather than erroring — the caller (IAM surface / boot seed) is the
/// authority on which slugs are valid.
#[cfg(feature = "ssr")]
pub async fn create_user(
    pool: &Pool,
    username: &str,
    password_hash: &str,
    roles: &[String],
) -> Result<Uuid> {
    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_one(
            "INSERT INTO public.users (username, password_hash, status)
             VALUES ($1, $2, 'active')
             RETURNING id",
            &[&username, &password_hash],
        )
        .await
        .with_context(|| format!("create user {username}"))?;
    let user_id: Uuid = row.get(0);

    for slug in roles {
        client
            .execute(
                "INSERT INTO public.role_grants (user_id, role_id)
                 SELECT $1, id FROM public.roles WHERE slug = $2
                 ON CONFLICT (user_id, role_id) DO NOTHING",
                &[&user_id, slug],
            )
            .await
            .with_context(|| format!("grant {slug} to {username}"))?;
    }
    Ok(user_id)
}

/// A user summary row for the admin IAM surface (list view). `last_login_at`
/// is projected as an RFC3339 string so the DTO is feature-agnostic (the
/// `chrono` dep is ssr-only).
#[cfg(feature = "ssr")]
#[derive(Debug, Clone)]
pub struct UserListRow {
    pub id: Uuid,
    pub username: String,
    pub email: Option<String>,
    pub status: String,
    pub roles: Vec<String>,
    pub last_login_at: Option<String>,
}

/// List all users with their granted role slugs (admin IAM surface).
#[cfg(feature = "ssr")]
pub async fn list_users(pool: &Pool) -> Result<Vec<UserListRow>> {
    let client = pool.get().await.context("pg client")?;
    let rows = client
        .query(
            "SELECT u.id, u.username, u.email, u.status, u.last_login_at,
                    COALESCE(
                        ARRAY(
                            SELECT r.slug
                            FROM public.role_grants rg
                            JOIN public.roles r ON r.id = rg.role_id
                            WHERE rg.user_id = u.id
                            ORDER BY r.is_system DESC, r.slug
                        ),
                        ARRAY[]::TEXT[]
                    ) AS roles
             FROM public.users u
             ORDER BY u.username",
            &[],
        )
        .await
        .context("list_users")?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let last_login: Option<DateTime<Utc>> = r.get(4);
            UserListRow {
                id: r.get(0),
                username: r.get(1),
                email: r.get(2),
                status: r.get(3),
                last_login_at: last_login.map(|t| t.to_rfc3339()),
                roles: r.get(5),
            }
        })
        .collect())
}

/// Set a user's status AND, in the same transaction, delete all their live
/// sessions. Disabling a user must both bar future logins (via find_user's
/// status gate) and immediately revoke in-flight sessions (FR-IAM-13) — the
/// two writes must be atomic so no window exists where the account is disabled
/// but a session survives.
#[cfg(feature = "ssr")]
pub async fn set_user_status(pool: &Pool, user_id: Uuid, status: &str) -> Result<()> {
    let mut client = pool.get().await.context("pg client")?;
    let tx = client.transaction().await.context("begin tx")?;
    tx.execute(
        "UPDATE public.users SET status = $2 WHERE id = $1",
        &[&user_id, &status],
    )
    .await
    .context("update status")?;
    tx.execute(
        "DELETE FROM public.sessions WHERE user_id = $1",
        &[&user_id],
    )
    .await
    .context("revoke sessions")?;
    tx.commit().await.context("commit set_user_status")?;
    Ok(())
}

/// Grant a role slug to a user (idempotent).
#[cfg(feature = "ssr")]
pub async fn assign_role(pool: &Pool, user_id: Uuid, slug: &str) -> Result<()> {
    let client = pool.get().await.context("pg client")?;
    client
        .execute(
            "INSERT INTO public.role_grants (user_id, role_id)
             SELECT $1, id FROM public.roles WHERE slug = $2
             ON CONFLICT (user_id, role_id) DO NOTHING",
            &[&user_id, &slug],
        )
        .await
        .with_context(|| format!("assign_role {slug}"))?;
    Ok(())
}

/// Revoke a role slug from a user.
#[cfg(feature = "ssr")]
pub async fn revoke_role(pool: &Pool, user_id: Uuid, slug: &str) -> Result<()> {
    let client = pool.get().await.context("pg client")?;
    client
        .execute(
            "DELETE FROM public.role_grants rg
             USING public.roles r
             WHERE rg.role_id = r.id AND rg.user_id = $1 AND r.slug = $2",
            &[&user_id, &slug],
        )
        .await
        .with_context(|| format!("revoke_role {slug}"))?;
    Ok(())
}

/// Count active users holding the `admin` role, EXCLUDING the given user id.
/// Backs the last-admin guard (FR-IAM-14): disabling an admin or revoking the
/// admin role must refuse if it would drop the enabled-admin count to zero.
#[cfg(feature = "ssr")]
pub async fn enabled_admin_count_excluding(pool: &Pool, user_id: Uuid) -> Result<i64> {
    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_one(
            "SELECT count(*) FROM public.users u
               JOIN public.role_grants rg ON rg.user_id = u.id
               JOIN public.roles r        ON r.id = rg.role_id
              WHERE r.slug = 'admin' AND u.status = 'active' AND u.id <> $1",
            &[&user_id],
        )
        .await
        .context("enabled_admin_count_excluding")?;
    Ok(row.get(0))
}

// ---- gateway identities (P4, FR-GW-08) --------------------------------------

/// A gateway identity binding resolved to its IAM actor + roles. This is the
/// actor a channel-originated turn runs under — the same shape a browser turn's
/// `Claims` yields, so a gateway turn traverses the identical Harness/Guard path
/// (FR-GW-09/10).
#[cfg(feature = "ssr")]
#[derive(Debug, Clone)]
pub struct GatewayActor {
    pub user_id: Uuid,
    pub username: String,
    pub roles: Vec<String>,
}

/// Resolve a `(channel, platform_handle)` to its bound IAM user + roles.
///
/// **FAIL-CLOSED (FR-GW-08).** A handle with no binding — or one bound to a
/// non-`active` account — resolves to `None`, and the caller MUST refuse the
/// message without dispatching any capability. Roles come from the same
/// `role_grants` join the console uses (`find_user`), never from the inbound
/// payload, so a channel cannot elevate an actor (FR-GW-10).
#[cfg(feature = "ssr")]
pub async fn resolve_gateway_identity(
    pool: &Pool,
    channel: &str,
    platform_handle: &str,
) -> Result<Option<GatewayActor>> {
    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_opt(
            "SELECT u.id, u.username,
                    COALESCE(
                        ARRAY(
                            SELECT r.slug
                            FROM public.role_grants rg
                            JOIN public.roles r ON r.id = rg.role_id
                            WHERE rg.user_id = u.id
                            ORDER BY r.is_system DESC, r.slug
                        ),
                        ARRAY[]::TEXT[]
                    ) AS roles
             FROM public.gateway_identities gi
             JOIN public.users u ON u.id = gi.user_id
             WHERE gi.channel = $1 AND gi.platform_handle = $2
               AND u.status = 'active'
             LIMIT 1",
            &[&channel, &platform_handle],
        )
        .await
        .context("resolve_gateway_identity")?;
    Ok(row.map(|r| GatewayActor {
        user_id: r.get(0),
        username: r.get(1),
        roles: r.get(2),
    }))
}

/// P7-2 (FR-GW-14): resolve an inbound reply to the approval it decides. Given
/// the channel + the provider id of the message being replied to, return the
/// approval id (the delivery row's `signature`) of the approval ALERT that was
/// sent as that message. `None` when the reply is not to a known approval alert.
#[cfg(feature = "ssr")]
pub async fn resolve_approval_by_reply(
    pool: &Pool,
    channel: &str,
    provider_message_id: &str,
) -> Result<Option<Uuid>> {
    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_opt(
            "SELECT signature
               FROM public.alert_deliveries
              WHERE channel = $1 AND provider_message_id = $2 AND alert_class = 'approval'
              ORDER BY created_at DESC
              LIMIT 1",
            &[&channel, &provider_message_id],
        )
        .await
        .context("resolve_approval_by_reply")?;
    Ok(row.and_then(|r| {
        let sig: String = r.get(0);
        Uuid::parse_str(&sig).ok()
    }))
}

/// P7-4: resolve a chat actor id (`user:<username>`) to the owning user's UUID,
/// for durable-history attribution. `None` (unknown user / db hiccup) → the
/// caller skips durable persistence and stays Redis-only (fail-soft).
#[cfg(feature = "ssr")]
pub async fn user_id_for_actor(pool: &Pool, actor_id: &str) -> Option<Uuid> {
    let username = actor_id.strip_prefix("user:").unwrap_or(actor_id);
    let client = pool.get().await.ok()?;
    let row = client
        .query_opt("SELECT id FROM public.users WHERE username = $1", &[&username])
        .await
        .ok()??;
    Some(row.get(0))
}

/// P7-4 (FR-UI-18): append one durable chat turn, upserting its session. The
/// session title is seeded from the first user turn (kept on later turns); a
/// re-save bumps `updated_at`. Best-effort at the call site — a DB hiccup must
/// not abort the chat turn (the Redis hot tier already served the operator).
#[cfg(feature = "ssr")]
pub async fn append_chat_turn(
    pool: &Pool,
    session_id: &str,
    owner_id: Uuid,
    role: &str,
    content: &str,
    tool_use_id: Option<&str>,
) -> Result<()> {
    let client = pool.get().await.context("pg client")?;
    // Seed the title only from the first user turn; ON CONFLICT keeps it.
    let title: String = if role == "user" {
        content.chars().take(60).collect()
    } else {
        String::new()
    };
    client
        .execute(
            "INSERT INTO public.chat_sessions (id, owner_id, title)
             VALUES ($1, $2, $3)
             ON CONFLICT (id) DO UPDATE SET updated_at = now()",
            &[&session_id, &owner_id, &title],
        )
        .await
        .context("upsert chat_session")?;
    client
        .execute(
            "INSERT INTO public.chat_turns (session_id, role, content, tool_use_id)
             VALUES ($1, $2, $3, $4)",
            &[&session_id, &role, &content, &tool_use_id],
        )
        .await
        .context("insert chat_turn")?;
    Ok(())
}

/// P7-6 (FR-UI-20): prune chat history older than the retention window (by last
/// activity, `updated_at`). Returns the number of sessions removed (turns cascade).
/// `retention_days == 0` means RETAIN FOREVER — a mis-set/unset window must never
/// wipe everything. Independent of the auth-session TTL and the Redis TTL.
#[cfg(feature = "ssr")]
pub async fn prune_chat_history(pool: &Pool, retention_days: u64) -> Result<u64> {
    if retention_days == 0 {
        return Ok(0);
    }
    let client = pool.get().await.context("pg client")?;
    let days = retention_days.min(i32::MAX as u64) as i32;
    let n = client
        .execute(
            "DELETE FROM public.chat_sessions
              WHERE updated_at < now() - make_interval(days => $1)",
            &[&days],
        )
        .await
        .context("prune chat history")?;
    Ok(n)
}

/// A gateway identity binding row for the admin Channels surface.
#[cfg(feature = "ssr")]
#[derive(Debug, Clone)]
pub struct GatewayIdentityRow {
    pub id: Uuid,
    pub channel: String,
    pub platform_handle: String,
    pub user_id: Uuid,
    pub username: String,
    pub enrolled_at: DateTime<Utc>,
}

/// Enrol a handle → user binding (admin-gated; the Channels tab). Returns the
/// new binding id. `UNIQUE(channel, platform_handle)` rejects a duplicate.
#[cfg(feature = "ssr")]
pub async fn create_gateway_identity(
    pool: &Pool,
    channel: &str,
    platform_handle: &str,
    user_id: Uuid,
    enrolled_by: Option<Uuid>,
) -> Result<Uuid> {
    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_one(
            "INSERT INTO public.gateway_identities (channel, platform_handle, user_id, enrolled_by)
             VALUES ($1, $2, $3, $4)
             RETURNING id",
            &[&channel, &platform_handle, &user_id, &enrolled_by],
        )
        .await
        .context("create_gateway_identity")?;
    Ok(row.get(0))
}

/// List all bindings (admin surface), most-recent first.
#[cfg(feature = "ssr")]
pub async fn list_gateway_identities(pool: &Pool) -> Result<Vec<GatewayIdentityRow>> {
    let client = pool.get().await.context("pg client")?;
    let rows = client
        .query(
            "SELECT gi.id, gi.channel, gi.platform_handle, gi.user_id, u.username, gi.enrolled_at
             FROM public.gateway_identities gi
             JOIN public.users u ON u.id = gi.user_id
             ORDER BY gi.enrolled_at DESC",
            &[],
        )
        .await
        .context("list_gateway_identities")?;
    Ok(rows
        .into_iter()
        .map(|r| GatewayIdentityRow {
            id: r.get(0),
            channel: r.get(1),
            platform_handle: r.get(2),
            user_id: r.get(3),
            username: r.get(4),
            enrolled_at: r.get(5),
        })
        .collect())
}

/// Revoke a binding by id (admin surface).
#[cfg(feature = "ssr")]
pub async fn delete_gateway_identity(pool: &Pool, id: Uuid) -> Result<()> {
    let client = pool.get().await.context("pg client")?;
    client
        .execute("DELETE FROM public.gateway_identities WHERE id = $1", &[&id])
        .await
        .context("delete_gateway_identity")?;
    Ok(())
}

// ---- sessions ---------------------------------------------------------------

#[cfg(feature = "ssr")]
pub async fn insert_session(
    pool: &Pool,
    id: &str,
    user_id: Uuid,
    expires_at: DateTime<Utc>,
) -> Result<()> {
    let client = pool.get().await.context("pg client")?;
    client
        .execute(
            "INSERT INTO public.sessions (id, user_id, expires_at)
             VALUES ($1, $2, $3)",
            &[&id, &user_id, &expires_at],
        )
        .await
        .with_context(|| format!("insert session {id}"))?;
    Ok(())
}

#[cfg(feature = "ssr")]
pub async fn find_valid_session(pool: &Pool, id: &str) -> Result<Option<Uuid>> {
    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_opt(
            "SELECT user_id FROM public.sessions
             WHERE id = $1 AND expires_at > now()",
            &[&id],
        )
        .await
        .context("find_valid_session")?;
    Ok(row.map(|r| r.get::<_, Uuid>(0)))
}

#[cfg(feature = "ssr")]
pub async fn delete_session(pool: &Pool, id: &str) -> Result<()> {
    let client = pool.get().await.context("pg client")?;
    client
        .execute("DELETE FROM public.sessions WHERE id = $1", &[&id])
        .await
        .with_context(|| format!("delete session {id}"))?;
    Ok(())
}

// ---- app_config -------------------------------------------------------------
//
// Single-org: app_config is a key-only store — (key PK, JSONB value,
// is_secret, updated_at, updated_by). These legacy string helpers encode the
// value as a JSONB string.

#[cfg(feature = "ssr")]
pub async fn get_config(pool: &Pool, key: &str) -> Result<Option<String>> {
    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_opt(
            "SELECT value FROM public.app_config WHERE key = $1",
            &[&key],
        )
        .await
        .context("get_config")?;
    Ok(row.and_then(|r| {
        let v: serde_json::Value = r.get(0);
        v.as_str().map(String::from)
    }))
}

/// Read a config value as its raw JSON (not string-collapsed). `get_config`
/// above returns `None` for any non-string JSONB (e.g. a boolean
/// `channels.telegram.enabled = true`); boot-time consumers that need the real
/// type use this instead.
#[cfg(feature = "ssr")]
pub async fn get_config_json(pool: &Pool, key: &str) -> Result<Option<serde_json::Value>> {
    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_opt("SELECT value FROM public.app_config WHERE key = $1", &[&key])
        .await
        .context("get_config_json")?;
    Ok(row.map(|r| r.get::<_, serde_json::Value>(0)))
}

#[cfg(feature = "ssr")]
pub async fn set_config(pool: &Pool, key: &str, value: &str) -> Result<()> {
    let client = pool.get().await.context("pg client")?;
    let jval = serde_json::Value::String(value.to_string());
    client
        .execute(
            "INSERT INTO public.app_config (key, value)
             VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE
               SET value = EXCLUDED.value, updated_at = now()",
            &[&key, &jval],
        )
        .await
        .context("set_config")?;
    Ok(())
}

// ---- user_preferences -------------------------------------------------------

#[cfg(feature = "ssr")]
pub async fn get_preference(pool: &Pool, user_id: Uuid, key: &str) -> Result<Option<String>> {
    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_opt(
            "SELECT value FROM public.user_preferences
             WHERE user_id = $1 AND key = $2",
            &[&user_id, &key],
        )
        .await
        .context("get_preference")?;
    Ok(row.map(|r| r.get(0)))
}

#[cfg(feature = "ssr")]
pub async fn set_preference(
    pool: &Pool,
    user_id: Uuid,
    key: &str,
    value: &str,
) -> Result<()> {
    let client = pool.get().await.context("pg client")?;
    client
        .execute(
            "INSERT INTO public.user_preferences (user_id, key, value)
             VALUES ($1, $2, $3)
             ON CONFLICT (user_id, key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
            &[&user_id, &key, &value],
        )
        .await
        .with_context(|| format!("set_preference {key}"))?;
    Ok(())
}
