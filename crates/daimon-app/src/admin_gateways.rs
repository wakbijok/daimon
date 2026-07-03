//! Server fns for the Channels settings tab — gateway identity enrolment
//! (P4-6, FR-GW-08/16). All are `admin`-gated: a binding maps a chat-platform
//! handle to an IAM user, so it is an administrative act (an unmapped handle can
//! never mint an actor — the binding IS the authorization).

use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A gateway identity binding for the admin surface.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GatewayBindingDto {
    pub id: Uuid,
    pub channel: String,
    pub platform_handle: String,
    pub username: String,
    pub enrolled_at: String,
}

#[server]
pub async fn list_gateway_bindings() -> Result<Vec<GatewayBindingDto>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    require_admin().await?;
    let state = expect_context::<AppState>();
    let rows = crate::db::list_gateway_identities(&state.db)
        .await
        .map_err(|e| ServerFnError::new(format!("list_gateway_identities: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|r| GatewayBindingDto {
            id: r.id,
            channel: r.channel,
            platform_handle: r.platform_handle,
            username: r.username,
            enrolled_at: r.enrolled_at.to_rfc3339(),
        })
        .collect())
}

#[server]
pub async fn add_gateway_binding(
    channel: String,
    platform_handle: String,
    username: String,
) -> Result<(), ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();

    let channel = channel.trim().to_lowercase();
    let platform_handle = platform_handle.trim().to_string();
    let username = username.trim().to_string();
    if channel.is_empty() || platform_handle.is_empty() || username.is_empty() {
        return Err(ServerFnError::new(
            "channel, platform handle, and username are all required",
        ));
    }

    // Resolve the target IAM user — the binding must point at a real account.
    let user = crate::db::find_user(&state.db, &username)
        .await
        .map_err(|e| ServerFnError::new(format!("find_user: {e}")))?
        .ok_or_else(|| ServerFnError::new(format!("no such user: {username}")))?;

    crate::db::create_gateway_identity(
        &state.db,
        &channel,
        &platform_handle,
        user.id,
        Some(claims.user_id),
    )
    .await
    .map_err(|e| ServerFnError::new(format!("create_gateway_identity: {e}")))?;

    let mut meta = std::collections::BTreeMap::new();
    meta.insert("gateway.channel".to_string(), channel.clone());
    meta.insert("gateway.handle".to_string(), platform_handle.clone());
    meta.insert("gateway.user".to_string(), username.clone());
    let _ = state
        .broker
        .audit_admin_action(
            &claims.sub,
            daimon_broker::ActionKind::Other,
            None,
            None,
            Some(&format!(
                "gateway.identity.enrol {channel}:{platform_handle} -> {username}"
            )),
            meta,
        )
        .await;
    Ok(())
}

#[server]
pub async fn delete_gateway_binding(id: Uuid) -> Result<(), ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    crate::db::delete_gateway_identity(&state.db, id)
        .await
        .map_err(|e| ServerFnError::new(format!("delete_gateway_identity: {e}")))?;

    let mut meta = std::collections::BTreeMap::new();
    meta.insert("gateway.binding_id".to_string(), id.to_string());
    let _ = state
        .broker
        .audit_admin_action(
            &claims.sub,
            daimon_broker::ActionKind::Other,
            None,
            None,
            Some("gateway.identity.revoke"),
            meta,
        )
        .await;
    Ok(())
}
