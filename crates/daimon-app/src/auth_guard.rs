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
    let conn = state.db.lock().await;
    if db::find_valid_session(&conn, &claims.session_id).is_none() {
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

    let conn = state.db.lock().await;
    if db::find_valid_session(&conn, &claims.session_id).is_none() {
        return Err(unauthenticated());
    }

    Ok(claims)
}

/// Resolve the current request's JWT + session AND enforce `role == "admin"`.
///
/// Server-only helper called from inside `#[server]` functions backing
/// `/admin/*` routes. Returns:
/// - `Ok(claims)` — caller is authenticated AND has admin role
/// - `Err(ServerFnError::ServerError("unauthenticated"))` — no valid session
/// - `Err(ServerFnError::ServerError("forbidden"))` — session valid but role != "admin"
///
/// Used by Phase 2b admin surfaces (`/admin/credentials`, `/admin/targets`,
/// `/admin/audit`). Per D24, this is the single gate every admin server-fn
/// passes through.
#[cfg(feature = "ssr")]
pub async fn require_admin() -> Result<crate::auth::Claims, ServerFnError> {
    fn forbidden() -> ServerFnError {
        ServerFnError::ServerError("forbidden".into())
    }
    let claims = require_authenticated().await?;
    if claims.role != "admin" {
        tracing::warn!(
            actor = %claims.sub,
            role = %claims.role,
            "admin route denied — caller is authenticated but not admin"
        );
        return Err(forbidden());
    }
    Ok(claims)
}
