//! Postgres-backed app data layer (Phase 2c D3b).
//!
//! Async helpers against `daimon_db::Pool`. Replaces the prior rusqlite-on-
//! tokio-Mutex implementation. The same set of operations the SQLite version
//! provided (find_user, create_user, sessions, app_config, clusters, prefs).
//!
//! IDs are UUIDs; tenant scoping is wired through the function signature
//! where it matters (clusters, prefs, user-create). D6 will replace the
//! hardcoded default-tenant pattern at the call site with the JWT tenant
//! claim.

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

/// Resolve a tenant slug to its UUID. Default-tenant default for Phase 2c
/// single-tenant deployments; D6 plumbs per-request tenant routing.
#[cfg(feature = "ssr")]
pub async fn resolve_tenant_id(pool: &Pool, slug: &str) -> Result<Uuid> {
    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_one("SELECT id FROM public.tenants WHERE slug = $1", &[&slug])
        .await
        .with_context(|| format!("tenant lookup {slug}"))?;
    Ok(row.get(0))
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
            "SELECT u.id, u.username, u.password_hash, u.tenant_id,
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
    Ok(row.map(|r| {
        let tenant_opt: Option<Uuid> = r.get(3);
        UserRow {
            id: r.get(0),
            username: r.get(1),
            password_hash: r.get(2),
            tenant_id: tenant_opt.unwrap_or_else(Uuid::nil),
            roles: r.get(4),
        }
    }))
}

#[cfg(feature = "ssr")]
pub async fn create_user(
    pool: &Pool,
    tenant_id: Uuid,
    username: &str,
    password_hash: &str,
) -> Result<Uuid> {
    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_one(
            "INSERT INTO public.users (tenant_id, username, password_hash, status)
             VALUES ($1, $2, $3, 'active')
             RETURNING id",
            &[&tenant_id, &username, &password_hash],
        )
        .await
        .with_context(|| format!("create user {username}"))?;
    let user_id: Uuid = row.get(0);

    let role_row = client
        .query_one(
            "SELECT id FROM public.roles WHERE slug = 'tenant_admin'",
            &[],
        )
        .await
        .context("tenant_admin role lookup")?;
    let role_id: Uuid = role_row.get(0);
    let scope = format!("tenant:{tenant_id}");
    client
        .execute(
            "INSERT INTO public.role_grants (user_id, role_id, scope)
             VALUES ($1, $2, $3)
             ON CONFLICT (user_id, role_id, scope) DO NOTHING",
            &[&user_id, &role_id, &scope],
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
    Ok(row.map(|r| r.get(0)))
}

#[cfg(feature = "ssr")]
pub async fn set_config(pool: &Pool, key: &str, value: &str) -> Result<()> {
    let client = pool.get().await.context("pg client")?;
    client
        .execute(
            "INSERT INTO public.app_config (key, value)
             VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
            &[&key, &value],
        )
        .await
        .context("set_config")?;
    Ok(())
}

// ---- clusters ---------------------------------------------------------------

#[cfg(feature = "ssr")]
pub async fn list_clusters(pool: &Pool, tenant_id: Uuid) -> Result<Vec<(String, String)>> {
    let client = pool.get().await.context("pg client")?;
    let rows = client
        .query(
            "SELECT id, name FROM public.clusters
             WHERE tenant_id = $1
             ORDER BY name",
            &[&tenant_id],
        )
        .await
        .context("list_clusters")?;
    Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
}

#[cfg(feature = "ssr")]
#[derive(Debug, Clone)]
pub struct ClusterRow {
    pub id: String,
    pub name: String,
    pub api_url: String,
    pub token: String,
    pub notes: String,
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "ssr")]
pub async fn get_cluster(
    pool: &Pool,
    tenant_id: Uuid,
    id: &str,
) -> Result<Option<ClusterRow>> {
    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_opt(
            "SELECT id, name, api_url, token, notes, created_at
             FROM public.clusters
             WHERE tenant_id = $1 AND id = $2",
            &[&tenant_id, &id],
        )
        .await
        .with_context(|| format!("get_cluster {id}"))?;
    Ok(row.map(|r| ClusterRow {
        id: r.get(0),
        name: r.get(1),
        api_url: r.get(2),
        token: r.get(3),
        notes: r.get(4),
        created_at: r.get(5),
    }))
}

#[cfg(feature = "ssr")]
pub async fn insert_cluster(
    pool: &Pool,
    tenant_id: Uuid,
    id: &str,
    name: &str,
    api_url: &str,
    token: &str,
    notes: &str,
) -> Result<()> {
    let client = pool.get().await.context("pg client")?;
    client
        .execute(
            "INSERT INTO public.clusters
                (id, tenant_id, name, api_url, token, notes)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[&id, &tenant_id, &name, &api_url, &token, &notes],
        )
        .await
        .with_context(|| format!("insert cluster {id}"))?;
    Ok(())
}

#[cfg(feature = "ssr")]
pub async fn delete_cluster(pool: &Pool, tenant_id: Uuid, id: &str) -> Result<()> {
    let client = pool.get().await.context("pg client")?;
    client
        .execute(
            "DELETE FROM public.clusters WHERE tenant_id = $1 AND id = $2",
            &[&tenant_id, &id],
        )
        .await
        .with_context(|| format!("delete cluster {id}"))?;
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
