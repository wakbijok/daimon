use leptos::prelude::*;

#[server]
pub async fn get_current_user() -> Result<Option<(String, String)>, ServerFnError> {
    use crate::state::AppState;
    use crate::auth;
    use crate::db;
    use axum::http::header::COOKIE;
    use axum::http::request::Parts;

    let state = expect_context::<AppState>();

    // Extract request parts via leptos_axum
    let parts: Parts = leptos_axum::extract().await?;
    let cookie = parts
        .headers
        .get(COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Extract daimon_token from cookie string
    let token = cookie
        .split(';')
        .find_map(|c| c.trim().strip_prefix("daimon_token="));

    let Some(token) = token else {
        return Ok(None);
    };

    // Validate JWT
    let Some(claims) = auth::validate_jwt(&state.jwt_secret, token) else {
        return Ok(None);
    };

    // Validate session exists and not expired
    let sess = db::find_valid_session(&state.db, &claims.session_id)
        .await
        .map_err(|e| ServerFnError::new(format!("session lookup: {e}")))?;
    if sess.is_none() {
        return Ok(None);
    }

    Ok(Some((claims.sub, claims.role)))
}

/// Resolve the current request's JWT + session and return the claims.
///
/// Server-only helper, called from inside other `#[server]` functions to
/// enforce authentication. Returns:
/// - `Ok(claims)` — caller is authenticated; session is valid
/// - `Err(ServerFnError::ServerError("unauthenticated"))` — no/bad token
///   OR session expired
///
/// This does NOT check role — see [`require_admin`] for admin-gated routes.
#[cfg(feature = "ssr")]
pub async fn require_authenticated() -> Result<crate::auth::Claims, ServerFnError> {
    use crate::auth;
    use crate::db;
    use crate::state::AppState;
    use axum::http::header::COOKIE;
    use axum::http::request::Parts;

    fn unauthenticated() -> ServerFnError {
        ServerFnError::ServerError("unauthenticated".into())
    }

    let state = expect_context::<AppState>();
    let parts: Parts = leptos_axum::extract().await?;
    let cookie = parts
        .headers
        .get(COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let token = cookie
        .split(';')
        .find_map(|c| c.trim().strip_prefix("daimon_token="))
        .ok_or_else(unauthenticated)?;

    let claims = auth::validate_jwt(&state.jwt_secret, token).ok_or_else(unauthenticated)?;

    let sess = db::find_valid_session(&state.db, &claims.session_id)
        .await
        .map_err(|_| unauthenticated())?;
    if sess.is_none() {
        return Err(unauthenticated());
    }

    Ok(claims)
}

/// Resolve the current request's JWT + session AND enforce that the caller
/// holds at least one role permitting administration.
///
/// Phase 2c.D6: `admin` is no longer a single string. Callers with any of
/// `tenant_admin`, `cluster_admin`, or the legacy `admin` slug pass. Other
/// roles (operator, viewer, auditor) are rejected.
#[cfg(feature = "ssr")]
pub async fn require_admin() -> Result<crate::auth::Claims, ServerFnError> {
    fn forbidden() -> ServerFnError {
        ServerFnError::ServerError("forbidden".into())
    }
    let claims = require_authenticated().await?;
    let is_admin = claims.roles.iter().any(|r| matches!(r.as_str(),
        "tenant_admin" | "cluster_admin" | "admin"));
    if !is_admin {
        tracing::warn!(
            actor = %claims.sub,
            roles = ?claims.roles,
            "admin route denied — caller has no admin-class role"
        );
        return Err(forbidden());
    }
    Ok(claims)
}

/// Enforce a specific role slug. Cluster admins satisfy any role check.
#[cfg(feature = "ssr")]
pub async fn require_role(slug: &str) -> Result<crate::auth::Claims, ServerFnError> {
    fn forbidden() -> ServerFnError {
        ServerFnError::ServerError("forbidden".into())
    }
    let claims = require_authenticated().await?;
    let ok = claims
        .roles
        .iter()
        .any(|r| r == slug || r == "cluster_admin");
    if !ok {
        tracing::warn!(
            actor = %claims.sub,
            required = %slug,
            roles = ?claims.roles,
            "role check denied"
        );
        return Err(forbidden());
    }
    Ok(claims)
}

/// Enforce that the caller holds `cluster_admin`. Used by cross-tenant
/// management surfaces (tenant CRUD, system role config).
#[cfg(feature = "ssr")]
pub async fn require_cluster_admin() -> Result<crate::auth::Claims, ServerFnError> {
    fn forbidden() -> ServerFnError {
        ServerFnError::ServerError("forbidden".into())
    }
    let claims = require_authenticated().await?;
    if !claims.roles.iter().any(|r| r == "cluster_admin") {
        tracing::warn!(
            actor = %claims.sub,
            roles = ?claims.roles,
            "cluster_admin route denied"
        );
        return Err(forbidden());
    }
    Ok(claims)
}
