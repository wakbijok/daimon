//! WebSocket types, handler, and subscription manager for real-time PVE data.

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
        /// LLM client's default applies.
        #[serde(default)]
        model: Option<String>,
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

/// Subscription scope — identifies what data a client wants
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind")]
pub enum WsScope {
    ClusterResources {
        cluster_id: String,
    },
    NodeRrd {
        cluster_id: String,
        node: String,
    },
    GuestRrd {
        cluster_id: String,
        node: String,
        vmid: u32,
    },
    StorageRrd {
        cluster_id: String,
        node: String,
        storage: String,
    },
}

// ---- WebSocket handler (SSR only) ----

#[cfg(feature = "ssr")]
pub async fn ws_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    axum::Extension(state): axum::Extension<crate::state::AppState>,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

#[cfg(feature = "ssr")]
async fn handle_ws(mut socket: WebSocket, state: crate::state::AppState) {
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
                                    // Send initial snapshot from cache
                                    let cache = state.pve_cache.read().await;
                                    if let WsScope::ClusterResources { ref cluster_id } = scope {
                                        if let Some(resources) = cache.resources.get(cluster_id) {
                                            let snapshot = WsServerMsg::Snapshot {
                                                scope: scope.clone(),
                                                data: serde_json::to_value(resources).unwrap_or_default(),
                                            };
                                            let _ = socket.send(Message::Text(
                                                serde_json::to_string(&snapshot).unwrap_or_default().into()
                                            )).await;
                                        }
                                    }
                                    subscriptions.insert(scope);
                                }
                                WsClientMsg::Unsubscribe { scope } => {
                                    subscriptions.remove(&scope);
                                }
                                WsClientMsg::Ping => {
                                    let pong = serde_json::to_string(&WsServerMsg::Pong).unwrap_or_default();
                                    let _ = socket.send(Message::Text(pong.into())).await;
                                }
                                WsClientMsg::ChatSend { session_id, user_message, model } => {
                                    let actor = "operator"; // Phase 4.1 plumbs auth claims here
                                    crate::chat::handle_chat_send(
                                        &mut socket,
                                        &state,
                                        actor,
                                        session_id,
                                        user_message,
                                        model,
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
            scope: WsScope::ClusterResources {
                cluster_id: "pve1".to_string(),
            },
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "Subscribe");
        assert_eq!(json["scope"]["kind"], "ClusterResources");
        assert_eq!(json["scope"]["cluster_id"], "pve1");
    }

    #[test]
    fn snapshot_message_serializes_correctly() {
        let msg = WsServerMsg::Snapshot {
            scope: WsScope::NodeRrd {
                cluster_id: "pve1".to_string(),
                node: "node2".to_string(),
            },
            data: serde_json::json!({"cpu": 0.42}),
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "Snapshot");
        assert_eq!(json["scope"]["kind"], "NodeRrd");
        assert_eq!(json["scope"]["node"], "node2");
        assert_eq!(json["data"]["cpu"], 0.42);
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

    #[test]
    fn ws_scope_equality() {
        let a = WsScope::ClusterResources {
            cluster_id: "pve1".to_string(),
        };
        let b = WsScope::ClusterResources {
            cluster_id: "pve1".to_string(),
        };
        let c = WsScope::ClusterResources {
            cluster_id: "pve2".to_string(),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);

        let d = WsScope::GuestRrd {
            cluster_id: "pve1".to_string(),
            node: "node1".to_string(),
            vmid: 100,
        };
        let e = WsScope::GuestRrd {
            cluster_id: "pve1".to_string(),
            node: "node1".to_string(),
            vmid: 100,
        };
        let f = WsScope::GuestRrd {
            cluster_id: "pve1".to_string(),
            node: "node1".to_string(),
            vmid: 200,
        };
        assert_eq!(d, e);
        assert_ne!(d, f);
        assert_ne!(a, d); // different variants
    }
}
