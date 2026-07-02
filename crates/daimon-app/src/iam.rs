//! P1 — admin-gated IAM surface (`iam::*` server-fns) + `logout`.
//!
//! Every mutating fn calls `require_admin()` first (FR-IAM-11/12) and audits
//! the change on the D23 hash-chained global audit trail with the acting admin
//! as actor (FR-IAM-17) — the same `broker.audit_admin_action` pattern
//! `admin_settings.rs` uses. `set_user_status("disabled")` and
//! `revoke_role("admin")` additionally enforce the last-admin guard
//! (FR-IAM-14): they refuse if the mutation would drop the enabled-admin count
//! to zero, preventing control-plane lockout of the single-org system.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Wire summary of a user for the admin list view. `last_login_at` is an
/// RFC3339 string (feature-agnostic — `chrono` is ssr-only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserSummary {
    pub id: Uuid,
    pub username: String,
    pub email: Option<String>,
    pub status: String,
    pub roles: Vec<String>,
    pub last_login_at: Option<String>,
}

#[server]
pub async fn list_users() -> Result<Vec<UserSummary>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let _claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let rows = crate::db::list_users(&state.db)
        .await
        .map_err(|e| ServerFnError::new(format!("list_users: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|r| UserSummary {
            id: r.id,
            username: r.username,
            email: r.email,
            status: r.status,
            roles: r.roles,
            last_login_at: r.last_login_at,
        })
        .collect())
}

#[server]
pub async fn create_user(
    username: String,
    password: String,
    roles: Vec<String>,
) -> Result<Uuid, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();

    if username.trim().is_empty() {
        return Err(ServerFnError::new("username is required"));
    }
    if password.is_empty() {
        return Err(ServerFnError::new("password is required"));
    }

    let hash = crate::auth::hash_password(&password);
    let user_id = crate::db::create_user(&state.db, &username, &hash, &roles)
        .await
        .map_err(|e| ServerFnError::new(format!("create_user: {e}")))?;

    audit_iam(
        &state,
        &claims.sub,
        "user.create",
        user_id,
        format!("user.create {username} roles={roles:?}"),
    )
    .await;

    Ok(user_id)
}

#[server]
pub async fn set_user_status(user_id: Uuid, status: String) -> Result<(), ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();

    // Last-admin guard (FR-IAM-14): disabling a user must not leave zero
    // enabled admins.
    if status == "disabled" {
        let remaining = crate::db::enabled_admin_count_excluding(&state.db, user_id)
            .await
            .map_err(|e| ServerFnError::new(format!("admin count: {e}")))?;
        if remaining == 0 {
            return Err(ServerFnError::new(
                "cannot disable the last enabled admin",
            ));
        }
    }

    crate::db::set_user_status(&state.db, user_id, &status)
        .await
        .map_err(|e| ServerFnError::new(format!("set_user_status: {e}")))?;

    let action = if status == "disabled" {
        "user.disable"
    } else {
        "user.status"
    };
    audit_iam(
        &state,
        &claims.sub,
        action,
        user_id,
        format!("user.status {status}"),
    )
    .await;

    Ok(())
}

#[server]
pub async fn assign_role(user_id: Uuid, role_slug: String) -> Result<(), ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();

    crate::db::assign_role(&state.db, user_id, &role_slug)
        .await
        .map_err(|e| ServerFnError::new(format!("assign_role: {e}")))?;

    audit_iam(
        &state,
        &claims.sub,
        "role.assign",
        user_id,
        format!("role.assign {role_slug}"),
    )
    .await;

    Ok(())
}

#[server]
pub async fn revoke_role(user_id: Uuid, role_slug: String) -> Result<(), ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();

    // Last-admin guard (FR-IAM-14): revoking `admin` must not leave zero
    // enabled admins.
    if role_slug == "admin" {
        let remaining = crate::db::enabled_admin_count_excluding(&state.db, user_id)
            .await
            .map_err(|e| ServerFnError::new(format!("admin count: {e}")))?;
        if remaining == 0 {
            return Err(ServerFnError::new(
                "cannot revoke the last enabled admin",
            ));
        }
    }

    crate::db::revoke_role(&state.db, user_id, &role_slug)
        .await
        .map_err(|e| ServerFnError::new(format!("revoke_role: {e}")))?;

    audit_iam(
        &state,
        &claims.sub,
        "role.revoke",
        user_id,
        format!("role.revoke {role_slug}"),
    )
    .await;

    Ok(())
}

/// Real logout (FR-IAM-19, NFR-SEC-03): resolve the caller's session from the
/// validated JWT, delete the `public.sessions` row (so any surviving token
/// fails `find_valid_session` and is 401 on the next request), and expire the
/// cookie server-side.
#[server]
pub async fn logout() -> Result<(), ServerFnError> {
    use crate::auth_guard::require_authenticated;
    use crate::state::AppState;
    use axum::http::header::SET_COOKIE;

    let claims = require_authenticated().await?;
    let state = expect_context::<AppState>();
    crate::db::delete_session(&state.db, &claims.session_id)
        .await
        .map_err(|e| ServerFnError::new(format!("delete_session: {e}")))?;

    let opts = expect_context::<leptos_axum::ResponseOptions>();
    opts.insert_header(
        SET_COOKIE,
        "daimon_token=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
            .parse()
            .unwrap(),
    );
    Ok(())
}

/// Write an IAM mutation to the D23 global audit chain. Best-effort — the
/// mutation has already committed; a failed audit append is logged but does
/// not roll back the change (matches `admin_settings.rs`).
#[cfg(feature = "ssr")]
async fn audit_iam(
    state: &crate::state::AppState,
    actor: &str,
    action: &str,
    target_user: Uuid,
    op_summary: String,
) {
    let mut meta = std::collections::BTreeMap::new();
    meta.insert("iam.action".to_string(), action.to_string());
    meta.insert("iam.target_user".to_string(), target_user.to_string());
    let target_ref = format!("user:{target_user}");
    let _ = state
        .broker
        .audit_admin_action(
            actor,
            daimon_broker::ActionKind::Other,
            Some(&target_ref),
            None,
            Some(&op_summary),
            meta,
        )
        .await;
}
