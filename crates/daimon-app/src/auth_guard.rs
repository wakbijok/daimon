use leptos::prelude::*;

#[server]
pub async fn get_current_user() -> Result<Option<String>, ServerFnError> {
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
    let conn = state.db.lock().unwrap();
    if db::find_valid_session(&conn, &claims.session_id).is_none() {
        return Ok(None);
    }

    Ok(Some(claims.sub))
}
