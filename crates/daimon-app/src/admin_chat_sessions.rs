//! Chat session history server-fns (P7-5, FR-UI-18/19).
//!
//! History is now DURABLE + OWNER-SCOPED in Postgres (`chat_sessions` /
//! `chat_turns`, P7-3/4), replacing the pre-P7 localStorage index + Redis-only
//! read. Every read/list/delete authenticates and enforces OWNERSHIP
//! server-side: a user sees and resumes only their own sessions; an
//! `admin`/`auditor` may READ others' (oversight), but only the owner or an
//! `admin` may DELETE. Redis stays the hot tier and is wiped alongside a delete.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurnDto {
    pub role: String,
    pub content: String,
    pub tool_use_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSummaryDto {
    pub id: String,
    pub title: String,
    pub updated_at: String,
}

/// P7-5 (FR-UI-19): the caller's OWN sessions, most-recent first. Sourced from
/// Postgres, not localStorage — so the list follows the user across browsers.
#[server]
pub async fn list_my_sessions() -> Result<Vec<SessionSummaryDto>, ServerFnError> {
    use crate::auth_guard::require_authenticated;
    use crate::state::AppState;

    let claims = require_authenticated().await?;
    let state = expect_context::<AppState>();
    let client = state
        .db
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool: {e}")))?;
    let rows = client
        .query(
            "SELECT id, title, updated_at
               FROM public.chat_sessions
              WHERE owner_id = $1
              ORDER BY updated_at DESC
              LIMIT 200",
            &[&claims.user_id],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("list sessions: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let updated: chrono::DateTime<chrono::Utc> = r.get(2);
            SessionSummaryDto {
                id: r.get(0),
                title: r.get(1),
                updated_at: updated.to_rfc3339(),
            }
        })
        .collect())
}

/// P7-7 (FR-UI-05/15): the models the operator may select for a chat session —
/// the SAME permitted set the server-side turn validation enforces, so the
/// picker can only offer what will actually be honoured. Sourced from config at
/// runtime, never a compile-time list.
#[server]
pub async fn list_available_models() -> Result<Vec<String>, ServerFnError> {
    use crate::auth_guard::require_authenticated;
    use crate::state::AppState;

    require_authenticated().await?;
    let state = expect_context::<AppState>();
    Ok(crate::chat::permitted_models(&state.config.current()))
}

/// Owner of a session, or `None` if the session does not exist.
#[cfg(feature = "ssr")]
async fn session_owner(
    state: &crate::state::AppState,
    session_id: &str,
) -> Result<Option<uuid::Uuid>, ServerFnError> {
    let client = state
        .db
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool: {e}")))?;
    let row = client
        .query_opt(
            "SELECT owner_id FROM public.chat_sessions WHERE id = $1",
            &[&session_id],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("owner lookup: {e}")))?;
    Ok(row.map(|r| r.get(0)))
}

/// P7-5 (FR-UI-19): load a session's turns from durable history. Ownership is
/// verified server-side: the owner, or an `admin`/`auditor` (read-override).
#[server]
pub async fn load_chat_session(session_id: String) -> Result<Vec<ChatTurnDto>, ServerFnError> {
    use crate::auth_guard::require_authenticated;
    use crate::state::AppState;

    let claims = require_authenticated().await?;
    let state = expect_context::<AppState>();

    // Default-deny: a session that exists must be owned by the caller, unless the
    // caller holds a read-override role. A missing session returns empty (no
    // existence oracle beyond the caller's own namespace).
    match session_owner(&state, &session_id).await? {
        Some(owner) => {
            let is_owner = owner == claims.user_id;
            let read_override = claims.roles.iter().any(|r| r == "admin" || r == "auditor");
            if !is_owner && !read_override {
                return Err(ServerFnError::new("not authorized for this session"));
            }
        }
        None => return Ok(Vec::new()),
    }

    let client = state
        .db
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool: {e}")))?;
    let rows = client
        .query(
            "SELECT role, content, tool_use_id
               FROM public.chat_turns
              WHERE session_id = $1
              ORDER BY id ASC
              LIMIT 500",
            &[&session_id],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("load turns: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|r| ChatTurnDto {
            role: r.get(0),
            content: r.get(1),
            tool_use_id: r.get(2),
        })
        .collect())
}

/// P7-5 (FR-UI-19): delete a session. Only the OWNER or an `admin` may delete
/// (an `auditor` is read-only). Removes the durable rows (cascade) + the Redis
/// hot copy.
#[server]
pub async fn delete_chat_session(session_id: String) -> Result<(), ServerFnError> {
    use crate::auth_guard::require_authenticated;
    use crate::state::AppState;

    let claims = require_authenticated().await?;
    let state = expect_context::<AppState>();

    match session_owner(&state, &session_id).await? {
        Some(owner) => {
            let is_owner = owner == claims.user_id;
            let is_admin = claims.roles.iter().any(|r| r == "admin");
            if !is_owner && !is_admin {
                return Err(ServerFnError::new("not authorized to delete this session"));
            }
        }
        // Nothing durable to delete; still clear any Redis remnant below.
        None => {}
    }

    let client = state
        .db
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool: {e}")))?;
    // chat_turns cascade on the session delete.
    client
        .execute(
            "DELETE FROM public.chat_sessions WHERE id = $1",
            &[&session_id],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("delete session: {e}")))?;

    // Best-effort wipe of the Redis hot copy.
    let _ = state.working_memory.kv_delete("chat", &session_id).await;
    Ok(())
}
