//! daimon.db → public.users + public.sessions + public.clusters +
//! public.app_config + public.user_preferences (+ role_grants from
//! users.role).
//!
//! ID mapping: SQLite users.id (INT) → Postgres public.users.id (UUID),
//! deterministic v5 from (tenant_id, username). Sessions resolve through
//! the mapping. The Phase-2b users.role text column becomes a role_grant
//! in Postgres.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use rusqlite::Connection;
use uuid::Uuid;

use crate::MigrateStats;

pub async fn migrate(
    pool: &Pool,
    sqlite_path: &Path,
    tenant_id: Uuid,
    dry_run: bool,
) -> Result<MigrateStats> {
    if !sqlite_path.exists() {
        tracing::info!(path = %sqlite_path.display(), "daimon.db not found, skipping app migrate");
        return Ok(MigrateStats::default());
    }

    let extracted = extract(sqlite_path, tenant_id)?;
    let mut stats = MigrateStats {
        read: extracted.read_total(),
        inserted: 0,
        skipped: 0,
    };

    if dry_run {
        return Ok(stats);
    }

    let client = pool.get().await?;

    // Lookup operator role uuid once (every imported user gets 'operator' if
    // their SQLite role was not admin; admins get tenant_admin).
    let role_rows = client
        .query("SELECT slug, id FROM public.roles", &[])
        .await?;
    let mut roles: HashMap<String, Uuid> = HashMap::new();
    for r in role_rows {
        let slug: String = r.get(0);
        let id: Uuid = r.get(1);
        roles.insert(slug, id);
    }

    // users
    for u in &extracted.users {
        let n = client
            .execute(
                "INSERT INTO public.users
                    (id, tenant_id, username, password_hash, status, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, 'active', $5, $6)
                 ON CONFLICT (COALESCE(tenant_id, '00000000-0000-0000-0000-000000000000'::uuid), username)
                 DO UPDATE SET password_hash = EXCLUDED.password_hash, updated_at = EXCLUDED.updated_at",
                &[
                    &u.id,
                    &tenant_id,
                    &u.username,
                    &u.password_hash,
                    &u.created_at,
                    &u.updated_at,
                ],
            )
            .await
            .with_context(|| format!("insert user {}", u.username))?;
        if n == 1 { stats.inserted += 1; } else { stats.skipped += 1; }

        let role_slug = if u.role == "admin" { "tenant_admin" } else { "operator" };
        let role_id = roles.get(role_slug).copied().ok_or_else(|| anyhow::anyhow!("role {role_slug} not seeded"))?;
        let scope = format!("tenant:{tenant_id}");
        let _ = client
            .execute(
                "INSERT INTO public.role_grants (user_id, role_id, scope)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (user_id, role_id, scope) DO NOTHING",
                &[&u.id, &role_id, &scope],
            )
            .await
            .with_context(|| format!("grant role to {}", u.username))?;
    }

    // sessions
    for s in &extracted.sessions {
        let n = client
            .execute(
                "INSERT INTO public.sessions (id, user_id, expires_at, created_at)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (id) DO NOTHING",
                &[&s.id, &s.user_id, &s.expires_at, &s.created_at],
            )
            .await
            .with_context(|| format!("insert session {}", s.id))?;
        if n == 1 { stats.inserted += 1; } else { stats.skipped += 1; }
    }

    // clusters
    for c in &extracted.clusters {
        let n = client
            .execute(
                "INSERT INTO public.clusters
                    (id, tenant_id, name, api_url, token, notes, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $7)
                 ON CONFLICT (id) DO UPDATE
                    SET name = EXCLUDED.name,
                        api_url = EXCLUDED.api_url,
                        token = EXCLUDED.token,
                        notes = EXCLUDED.notes,
                        updated_at = now()",
                &[
                    &c.id,
                    &tenant_id,
                    &c.name,
                    &c.api_url,
                    &c.token,
                    &c.notes,
                    &c.created_at,
                ],
            )
            .await
            .with_context(|| format!("insert cluster {}", c.name))?;
        if n == 1 { stats.inserted += 1; } else { stats.skipped += 1; }
    }

    // app_config
    for (k, v) in &extracted.config {
        let n = client
            .execute(
                "INSERT INTO public.app_config (key, value)
                 VALUES ($1, $2)
                 ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
                &[&k, &v],
            )
            .await
            .with_context(|| format!("insert config {k}"))?;
        if n == 1 { stats.inserted += 1; } else { stats.skipped += 1; }
    }

    // user_preferences
    for p in &extracted.preferences {
        let n = client
            .execute(
                "INSERT INTO public.user_preferences (user_id, key, value)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (user_id, key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
                &[&p.user_id, &p.key, &p.value],
            )
            .await
            .with_context(|| format!("insert pref {}/{}", p.user_id, p.key))?;
        if n == 1 { stats.inserted += 1; } else { stats.skipped += 1; }
    }

    tracing::info!(target: "migrate.app", ?stats, "done");
    Ok(stats)
}

struct Extracted {
    users: Vec<UserRow>,
    sessions: Vec<SessionRow>,
    clusters: Vec<ClusterRow>,
    config: Vec<(String, String)>,
    preferences: Vec<PrefRow>,
}

impl Extracted {
    fn read_total(&self) -> usize {
        self.users.len() + self.sessions.len() + self.clusters.len()
            + self.config.len() + self.preferences.len()
    }
}

struct UserRow {
    id: Uuid,
    username: String,
    password_hash: String,
    role: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

struct SessionRow {
    id: String,
    user_id: Uuid,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

struct ClusterRow {
    id: String,
    name: String,
    api_url: String,
    token: String,
    notes: String,
    created_at: DateTime<Utc>,
}

struct PrefRow {
    user_id: Uuid,
    key: String,
    value: String,
}

fn extract(sqlite_path: &Path, tenant_id: Uuid) -> Result<Extracted> {
    let conn = Connection::open(sqlite_path)?;

    // ---- users ----
    let mut user_stmt = conn.prepare(
        "SELECT id, username, password_hash, role, created_at, updated_at FROM users",
    )?;
    let raw_users: Vec<(i64, String, String, String, String, String)> = user_stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(user_stmt);
    let mut user_id_map: HashMap<i64, Uuid> = HashMap::new();
    let mut users = Vec::with_capacity(raw_users.len());
    for (sqlite_id, username, password_hash, role, created, updated) in raw_users {
        let id = derive_user_uuid(tenant_id, &username);
        user_id_map.insert(sqlite_id, id);
        users.push(UserRow {
            id,
            username,
            password_hash,
            role,
            created_at: parse_app_ts(&created)?,
            updated_at: parse_app_ts(&updated)?,
        });
    }

    // ---- sessions ----
    let mut sess_stmt = conn.prepare(
        "SELECT id, user_id, expires_at, created_at FROM sessions",
    )?;
    let raw_sess: Vec<(String, i64, String, String)> = sess_stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(sess_stmt);
    let mut sessions = Vec::new();
    for (id, user_id, expires_at, created_at) in raw_sess {
        let Some(user_uuid) = user_id_map.get(&user_id).copied() else {
            tracing::warn!(session = %id, user_id, "session references unknown user, skipping");
            continue;
        };
        let expires = parse_session_expires(&expires_at);
        sessions.push(SessionRow {
            id,
            user_id: user_uuid,
            expires_at: expires,
            created_at: parse_app_ts(&created_at)?,
        });
    }

    // ---- clusters ----
    let mut cluster_stmt = conn.prepare(
        "SELECT id, name, api_url, token, notes, created_at FROM clusters",
    )?;
    let raw_clusters: Vec<(String, String, String, String, String, String)> = cluster_stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(cluster_stmt);
    let mut clusters = Vec::with_capacity(raw_clusters.len());
    for (id, name, api_url, token, notes, created_at) in raw_clusters {
        clusters.push(ClusterRow {
            id,
            name,
            api_url,
            token,
            notes,
            created_at: parse_app_ts(&created_at)?,
        });
    }

    // ---- config ----
    let mut cfg_stmt = conn.prepare("SELECT key, value FROM config")?;
    let raw_cfg: Vec<(String, String)> = cfg_stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(cfg_stmt);

    // ---- user_preferences ----
    let mut pref_stmt = conn.prepare("SELECT user_id, key, value FROM user_preferences")?;
    let raw_prefs: Vec<(i64, String, String)> = pref_stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(pref_stmt);
    let mut prefs = Vec::new();
    for (uid, key, value) in raw_prefs {
        if let Some(user_uuid) = user_id_map.get(&uid).copied() {
            prefs.push(PrefRow { user_id: user_uuid, key, value });
        }
    }

    Ok(Extracted {
        users,
        sessions,
        clusters,
        config: raw_cfg,
        preferences: prefs,
    })
}

fn derive_user_uuid(tenant_id: Uuid, username: &str) -> Uuid {
    let input = format!("user/{tenant_id}/{username}");
    Uuid::new_v5(&Uuid::NAMESPACE_OID, input.as_bytes())
}

fn parse_app_ts(s: &str) -> Result<DateTime<Utc>> {
    // daimon-app stores via datetime('now') => "YYYY-MM-DD HH:MM:SS" (UTC, no tz)
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Ok(d.with_timezone(&Utc));
    }
    let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .with_context(|| format!("parse app ts {s}"))?;
    Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
}

fn parse_session_expires(s: &str) -> DateTime<Utc> {
    // Session expires_at is stored as unix-seconds string.
    if let Ok(epoch) = s.parse::<i64>() {
        if let Some(dt) = DateTime::<Utc>::from_timestamp(epoch, 0) {
            return dt;
        }
    }
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}
