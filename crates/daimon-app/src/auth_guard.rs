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

/// Cookie → JWT → live-session authenticator, factored out of
/// [`require_authenticated`] so the WebSocket upgrade handler (which has an
/// `axum::http::HeaderMap` rather than a leptos server-fn context) can share
/// the exact same three-step check: parse the `daimon_token` cookie, validate
/// the JWT, and require a live `public.sessions` row.
///
/// Returns `Some(claims)` only when all three succeed; `None` on any failure
/// (no/bad cookie, invalid JWT, dead/expired session). A valid JWT alone is
/// NOT sufficient — the session row is the real gate (NFR-SEC-03).
#[cfg(feature = "ssr")]
pub async fn authenticate_headers(
    state: &crate::state::AppState,
    headers: &axum::http::HeaderMap,
) -> Option<crate::auth::Claims> {
    let cookie = headers
        .get(axum::http::header::COOKIE)?
        .to_str()
        .ok()?;
    let token = cookie
        .split(';')
        .find_map(|c| c.trim().strip_prefix("daimon_token="))?;
    let claims = crate::auth::validate_jwt(&state.jwt_secret, token)?;
    // NFR-SEC-03: JWT alone is insufficient — require a live session row.
    crate::db::find_valid_session(&state.db, &claims.session_id)
        .await
        .ok()??;
    Some(claims)
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
/// Shares the cookie → JWT → session check with [`authenticate_headers`].
#[cfg(feature = "ssr")]
pub async fn require_authenticated() -> Result<crate::auth::Claims, ServerFnError> {
    use crate::state::AppState;
    use axum::http::request::Parts;

    fn unauthenticated() -> ServerFnError {
        ServerFnError::ServerError("unauthenticated".into())
    }

    let state = expect_context::<AppState>();
    let parts: Parts = leptos_axum::extract().await?;
    authenticate_headers(&state, &parts.headers)
        .await
        .ok_or_else(unauthenticated)
}

/// Resolve the current request's JWT + session AND enforce that the caller
/// holds the `admin` role.
///
/// Single-org (FR-IAM-02): `admin` is the one canonical administrator slug —
/// the legacy `cluster_admin` / `tenant_admin` slugs are gone (folded into
/// `admin` by migration V025). Any other role (operator, approver, read-only,
/// auditor) is rejected.
#[cfg(feature = "ssr")]
pub async fn require_admin() -> Result<crate::auth::Claims, ServerFnError> {
    fn forbidden() -> ServerFnError {
        ServerFnError::ServerError("forbidden".into())
    }
    let claims = require_authenticated().await?;
    if !claims.roles.iter().any(|r| r == "admin") {
        tracing::warn!(
            actor = %claims.sub,
            roles = ?claims.roles,
            "admin route denied — caller has no admin role"
        );
        return Err(forbidden());
    }
    Ok(claims)
}

/// Enforce a specific role slug.
///
/// FR-IAM-03: there is no blanket bypass. The only superset relation is the
/// narrow, explicit `admin ⊇ operator` — an admin may perform operator
/// actions. No other role is implied.
#[cfg(feature = "ssr")]
pub async fn require_role(slug: &str) -> Result<crate::auth::Claims, ServerFnError> {
    fn forbidden() -> ServerFnError {
        ServerFnError::ServerError("forbidden".into())
    }
    let claims = require_authenticated().await?;
    let ok = claims
        .roles
        .iter()
        .any(|r| r == slug || (slug == "operator" && r == "admin"));
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

/// Enforce that the caller may approve/deny guard-gated writes (FR-IAM-05).
///
/// Satisfied by the `approver` role, and by `admin` (admin ⊇ approver).
#[cfg(feature = "ssr")]
pub async fn require_approver() -> Result<crate::auth::Claims, ServerFnError> {
    fn forbidden() -> ServerFnError {
        ServerFnError::ServerError("forbidden".into())
    }
    let claims = require_authenticated().await?;
    let ok = claims
        .roles
        .iter()
        .any(|r| r == "approver" || r == "admin");
    if !ok {
        tracing::warn!(
            actor = %claims.sub,
            roles = ?claims.roles,
            "approver route denied — caller has no approver/admin role"
        );
        return Err(forbidden());
    }
    Ok(claims)
}
