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
    pub tenant_id: Uuid,
    pub roles: Vec<String>,
}

#[cfg(feature = "ssr")]
pub async fn find_user(pool: &Pool, username: &str) -> Result<Option<UserRow>> {
    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_opt(
            "SELECT u.id, u.username, u.password_hash,
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
        // users.tenant_id was dropped (single-org). Synthetic placeholder
        // so create_jwt / login keep compiling with no signature change.
        tenant_id: Uuid::nil(),
        roles: r.get(3),
    }))
}

#[cfg(feature = "ssr")]
pub async fn create_user(
    pool: &Pool,
    username: &str,
    password_hash: &str,
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

    // P1: remap tenant_admin -> admin
    let role_row = client
        .query_one(
            "SELECT id FROM public.roles WHERE slug = 'tenant_admin'",
            &[],
        )
        .await
        .context("tenant_admin role lookup")?;
    let role_id: Uuid = role_row.get(0);
    client
        .execute(
            "INSERT INTO public.role_grants (user_id, role_id)
             VALUES ($1, $2)
             ON CONFLICT (user_id, role_id) DO NOTHING",
            &[&user_id, &role_id],
        )
        .await
        .context("grant tenant_admin")?;
    Ok(user_id)
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
