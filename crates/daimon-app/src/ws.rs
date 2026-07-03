//! WebSocket types, handler, and subscription manager for real-time data
//! streaming and the chat surface.

#[cfg(feature = "ssr")]
use axum::extract::ws::{Message, WebSocket};
use serde::{Deserialize, Serialize};

// ---- Message types ----

/// Client -> Server messages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsClientMsg {
    Subscribe { scope: WsScope },
    Unsubscribe { scope: WsScope },
    Ping,
    /// Phase 4 D3 — open or continue a chat session. `session_id` ties
    /// subsequent ChatSend/AgentTokenDelta to the same conversation. The
    /// server handler keeps prior turns in working memory.
    ChatSend {
        session_id: String,
        user_message: String,
        /// Optional model override (e.g. "claude-opus-4-7"); otherwise the
        /// LLM client's default applies. VALIDATED server-side against the
        /// permitted set (P7-7) — an unpermitted model is rejected, not honoured.
        #[serde(default)]
        model: Option<String>,
        /// P7-7 (FR-UI-16): optional reasoning-effort tier (e.g. "fast" /
        /// "deliberate"); validated server-side, applied where the provider
        /// supports it, default-through otherwise.
        #[serde(default)]
        effort: Option<String>,
    },
}

/// Server -> Client messages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsServerMsg {
    Snapshot {
        scope: WsScope,
        data: serde_json::Value,
    },
    Update {
        scope: WsScope,
        data: serde_json::Value,
    },
    Pong,
    Error {
        message: String,
    },
    /// Phase 4 D3 — streamed token / content delta from an agent. Multiple
    /// deltas land before the matching `AgentDone`.
    AgentTokenDelta {
        agent_id: String,
        session_id: String,
        content_delta: String,
    },
    /// Phase 4 D3 — an LLM emitted a tool-use block; the server has
    /// dispatched it and is awaiting the worker reply. Surfaces in the UI
    /// as a "calling tool X with input Y" status row.
    AgentToolUse {
        agent_id: String,
        session_id: String,
        tool: String,
        params: serde_json::Value,
    },
    /// Phase 4 D3 — tool result the server fed back to the LLM, surfaced
    /// for the UI to render alongside the assistant message.
    AgentToolResult {
        agent_id: String,
        session_id: String,
        tool: String,
        output: String,
        is_error: bool,
    },
    /// Phase 4 D3 — the assistant turn completed (no more deltas for this
    /// session until the next ChatSend).
    AgentDone {
        agent_id: String,
        session_id: String,
        stop_reason: String,
        input_tokens: u32,
        output_tokens: u32,
    },
}

/// Subscription scope — identifies what data a client wants.
///
/// P4: real scopes rebuilt on generic inventory. The PVE-shaped variants
/// (ClusterResources / NodeRrd / GuestRrd / StorageRrd) were removed with the
/// single-org / PVE-removal cut; this placeholder keeps the type + handler
/// compiling until the generic inventory subscription model lands.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind")]
pub enum WsScope {
    Placeholder,
}

// ---- Browser ReplySink (SSR only) ----

/// The browser WebSocket as a `daimon_gateway::ReplySink` (FR-GW-01). This is the
/// streaming impl: every `TurnEvent` maps 1:1 to the `WsServerMsg` the turn used
/// to send directly, so the wire output on `/api/v1/ws` is unchanged — the
/// `chat_bubble.rs` client parses the identical JSON. `agent_id` is `"chat"`,
/// matching the pre-refactor emissions.
#[cfg(feature = "ssr")]
pub struct WsSink<'a> {
    pub socket: &'a mut WebSocket,
}

#[cfg(feature = "ssr")]
impl WsSink<'_> {
    async fn send_msg(&mut self, msg: WsServerMsg) {
        if let Ok(json) = serde_json::to_string(&msg) {
            let _ = self.socket.send(Message::Text(json.into())).await;
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait::async_trait]
impl daimon_gateway::ReplySink for WsSink<'_> {
    fn streams(&self) -> bool {
        true
    }

    async fn emit(&mut self, event: daimon_gateway::TurnEvent) {
        use daimon_gateway::TurnEvent;
        let msg = match event {
            TurnEvent::TokenDelta {
                session_id,
                content,
            } => WsServerMsg::AgentTokenDelta {
                agent_id: "chat".into(),
                session_id,
                content_delta: content,
            },
            TurnEvent::ToolUse {
                session_id,
                tool,
                params,
            } => WsServerMsg::AgentToolUse {
                agent_id: "chat".into(),
                session_id,
                tool,
                params,
            },
            TurnEvent::ToolResult {
                session_id,
                tool,
                output,
                is_error,
            } => WsServerMsg::AgentToolResult {
                agent_id: "chat".into(),
                session_id,
                tool,
                output,
                is_error,
            },
            TurnEvent::Done {
                session_id,
                stop_reason,
                input_tokens,
                output_tokens,
            } => WsServerMsg::AgentDone {
                agent_id: "chat".into(),
                session_id,
                stop_reason,
                input_tokens,
                output_tokens,
            },
            TurnEvent::Error { message } => WsServerMsg::Error { message },
        };
        self.send_msg(msg).await;
    }

    async fn finish(&mut self) {
        // Streaming sink — `AgentDone` was already emitted; nothing to flush.
    }
}

// ---- WebSocket handler (SSR only) ----

#[cfg(feature = "ssr")]
pub async fn ws_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    axum::Extension(state): axum::Extension<crate::state::AppState>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // C4: authenticate BEFORE upgrading — no anonymous socket ever exists.
    // The socket reaches the LLM + SSH tool dispatch, so this is a blocker.
    let Some(claims) = crate::auth_guard::authenticate_headers(&state, &headers).await else {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            "authentication required",
        )
            .into_response();
    };
    let actor = format!("user:{}", claims.sub);
    ws.on_upgrade(move |socket| handle_ws(socket, state, actor))
        .into_response()
}

#[cfg(feature = "ssr")]
async fn handle_ws(mut socket: WebSocket, state: crate::state::AppState, actor: String) {
    let mut rx = state.ws_broadcast.subscribe();
    let mut subscriptions: std::collections::HashSet<WsScope> = std::collections::HashSet::new();

    loop {
        tokio::select! {
            // Incoming messages from client
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(client_msg) = serde_json::from_str::<WsClientMsg>(&text) {
                            match client_msg {
                                WsClientMsg::Subscribe { scope } => {
                                    // P4: no cached snapshot to replay yet — the
                                    // PVE cache was removed with the single-org /
                                    // PVE-removal cut. Generic inventory
                                    // subscriptions rebuild this path.
                                    subscriptions.insert(scope);
                                }
                                WsClientMsg::Unsubscribe { scope } => {
                                    subscriptions.remove(&scope);
                                }
                                WsClientMsg::Ping => {
                                    let pong = serde_json::to_string(&WsServerMsg::Pong).unwrap_or_default();
                                    let _ = socket.send(Message::Text(pong.into())).await;
                                }
                                WsClientMsg::ChatSend { session_id, user_message, model, effort } => {
                                    // Real authenticated operator (C4/AC-P1-07),
                                    // not the old hardcoded "operator". P4: the
                                    // browser socket is now one `ReplySink` among
                                    // many — the turn loop writes into `WsSink`,
                                    // which reproduces today's `WsServerMsg` wire
                                    // output byte-for-byte (FR-GW-01/03).
                                    let mut sink = WsSink { socket: &mut socket };
                                    crate::chat::handle_chat_send(
                                        &mut sink,
                                        &state,
                                        &actor,
                                        session_id,
                                        user_message,
                                        model,
                                        effort,
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            // Broadcast updates from background poller
            update = rx.recv() => {
                if let Ok(text) = update {
                    // Only forward if client is subscribed to this scope
                    if let Ok(server_msg) = serde_json::from_str::<WsServerMsg>(&text) {
                        let scope = match &server_msg {
                            WsServerMsg::Update { scope, .. } => Some(scope),
                            WsServerMsg::Snapshot { scope, .. } => Some(scope),
                            _ => None,
                        };
                        if let Some(scope) = scope {
                            if subscriptions.contains(scope) {
                                let _ = socket.send(Message::Text(text.into())).await;
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_message_serializes_with_type_tag() {
        let msg = WsClientMsg::Subscribe {
            scope: WsScope::Placeholder,
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "Subscribe");
        assert_eq!(json["scope"]["kind"], "Placeholder");
    }

    #[test]
    fn ping_pong_roundtrip() {
        // Serialize Ping
        let ping = WsClientMsg::Ping;
        let ping_json = serde_json::to_string(&ping).unwrap();
        let deserialized: WsClientMsg = serde_json::from_str(&ping_json).unwrap();
        assert!(matches!(deserialized, WsClientMsg::Ping));

        // Serialize Pong
        let pong = WsServerMsg::Pong;
        let pong_json = serde_json::to_string(&pong).unwrap();
        let deserialized: WsServerMsg = serde_json::from_str(&pong_json).unwrap();
        assert!(matches!(deserialized, WsServerMsg::Pong));
    }
}
