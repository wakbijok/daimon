//! Phase 4 D3 — chat surface backed by daimon-llm + tool-use dispatch.
//!
//! Flow on `WsClientMsg::ChatSend`:
//! 1. Load conversation from working memory (in-process for now; Redis in 4.1)
//! 2. Append user message
//! 3. Stream-completion against the LLM with tool definitions injected
//! 4. As deltas arrive, emit `AgentTokenDelta` to the WS
//! 5. If the LLM emits tool_use blocks, dispatch each via `NetworkAgent.run()`,
//!    append the tool result, and re-prompt — loop until `stop_reason != ToolUse`
//! 6. Emit `AgentDone` with usage; persist final conversation
//!
//! Every LLM call + tool dispatch lands an audit event via the broker.

#![cfg(feature = "ssr")]

use axum::extract::ws::{Message, WebSocket};
use chrono::Utc;
use daimon_broker::ActionKind;
use daimon_llm::{
    AnthropicClient, AssistantContent, ChatMessage, CompletionRequest, ContentDelta, LlmClient,
    Role, StopReason, ToolDefinition,
};
use daimon_redis::ConvMessage;
use daimon_tool_network::NetworkRequest;
use futures::StreamExt;
use serde_json::{Value as Json, json};
use std::time::Instant;
use tracing::{debug, error, info};

use crate::state::AppState;
use crate::ws::WsServerMsg;

const SYSTEM_PROMPT: &str = "\
You are dAImon — an operator-facing infrastructure agent. You can read \
network device state via the tools provided. Be terse, technical, and \
direct. When the operator's intent maps to a tool, call it. If the result \
is empty, surface that clearly. Never invent device state.";

/// Build the tool catalog from the NetworkAgent's capabilities. Phase 6
/// will replace this with a broker-registry query.
fn network_tools() -> Vec<ToolDefinition> {
    let target_schema = json!({
        "type": "object",
        "properties": {
            "target_ref": {
                "type": "string",
                "description": "Inventory target reference (e.g. 'target://mikrotik-edge')"
            }
        },
        "required": ["target_ref"]
    });
    vec![
        ToolDefinition {
            name: "network.routeros.system_info".into(),
            description: "Read RouterOS device identity / system info via SSH.".into(),
            input_schema: target_schema.clone(),
        },
        ToolDefinition {
            name: "network.routeros.interface_list".into(),
            description: "List all interfaces on a RouterOS device.".into(),
            input_schema: target_schema.clone(),
        },
        ToolDefinition {
            name: "network.routeros.ip_addresses".into(),
            description: "List configured IP addresses on a RouterOS device.".into(),
            input_schema: target_schema.clone(),
        },
        ToolDefinition {
            name: "network.routeros.firewall_filter_list".into(),
            description: "List firewall filter rules on a RouterOS device.".into(),
            input_schema: target_schema.clone(),
        },
    ]
}

/// Entry point — handle a single ChatSend message from the client.
pub async fn handle_chat_send(
    socket: &mut WebSocket,
    state: &AppState,
    actor_id: &str,
    session_id: String,
    user_message: String,
    model: Option<String>,
) {
    // Resolve clients lazily — Phase 4 default is Anthropic.
    let llm = match AnthropicClient::from_env() {
        Ok(c) => c,
        Err(e) => {
            send_err(socket, format!("llm init: {e}")).await;
            return;
        }
    };
    let network_agent = state.network_agent.clone();
    let working = state.working_memory.clone();

    // Load (or start) the conversation history from the working memory tier.
    let recent = match working.conv_recent(&session_id, 64).await {
        Ok(r) => r,
        Err(e) => {
            send_err(socket, format!("conv_recent: {e}")).await;
            return;
        }
    };
    let mut history: Vec<ChatMessage> = recent.into_iter().map(conv_to_chat).collect();
    history.push(ChatMessage::user(user_message.clone()));

    // Persist the user turn right away — survives mid-turn crashes.
    let _ = working
        .conv_push(
            &session_id,
            ConvMessage {
                role: "user".into(),
                content: user_message.clone(),
                tool_use_id: None,
                ts: Utc::now(),
            },
        )
        .await;

    let tools = network_tools();
    let mut loop_count = 0;
    let max_loops = 8; // safety cap

    'outer: loop {
        loop_count += 1;
        if loop_count > max_loops {
            send_err(socket, "tool-use loop budget exhausted".into()).await;
            break;
        }

        let req = CompletionRequest {
            model: model.clone().unwrap_or_default(),
            messages: history.clone(),
            system: Some(SYSTEM_PROMPT.into()),
            max_tokens: 4096,
            temperature: Some(0.2),
            tools: tools.clone(),
            request_id: Some(session_id.clone()),
        };

        let start = Instant::now();
        let mut stream = match llm.complete_stream(req).await {
            Ok(s) => s,
            Err(e) => {
                send_err(socket, format!("llm stream: {e}")).await;
                break;
            }
        };

        // State for the current assistant turn.
        let mut accumulated_text = String::new();
        let mut pending_tool: Option<PendingToolCall> = None;
        let mut completed_tool_calls: Vec<daimon_llm::ToolCall> = Vec::new();
        let mut final_stop = StopReason::Error;
        let mut final_usage = daimon_llm::Usage::default();

        while let Some(item) = stream.next().await {
            match item {
                Ok(delta) => match delta {
                    ContentDelta::TextDelta { text } => {
                        accumulated_text.push_str(&text);
                        send_ws(
                            socket,
                            WsServerMsg::AgentTokenDelta {
                                agent_id: "chat".into(),
                                session_id: session_id.clone(),
                                content_delta: text,
                            },
                        )
                        .await;
                    }
                    ContentDelta::ToolUseStart { id, name } => {
                        pending_tool = Some(PendingToolCall {
                            id,
                            name,
                            input_json: String::new(),
                        });
                    }
                    ContentDelta::ToolUseInputDelta { partial_json } => {
                        if let Some(p) = pending_tool.as_mut() {
                            p.input_json.push_str(&partial_json);
                        }
                    }
                    ContentDelta::ToolUseStop { .. } => {
                        if let Some(p) = pending_tool.take() {
                            let input: Json = serde_json::from_str(&p.input_json).unwrap_or_else(
                                |e| {
                                    debug!(error = %e, raw = %p.input_json, "tool input not valid json");
                                    json!({})
                                },
                            );
                            send_ws(
                                socket,
                                WsServerMsg::AgentToolUse {
                                    agent_id: "chat".into(),
                                    session_id: session_id.clone(),
                                    tool: p.name.clone(),
                                    params: input.clone(),
                                },
                            )
                            .await;
                            completed_tool_calls.push(daimon_llm::ToolCall {
                                id: p.id,
                                name: p.name,
                                arguments: input,
                            });
                        }
                    }
                    ContentDelta::MessageStop { stop_reason, usage } => {
                        final_stop = stop_reason;
                        final_usage = usage;
                    }
                },
                Err(e) => {
                    error!(error = %e, "llm stream error");
                    send_err(socket, format!("stream: {e}")).await;
                    break 'outer;
                }
            }
        }

        // Persist this assistant turn to history + working memory.
        let assistant_text = accumulated_text.clone();
        let assistant_msg = ChatMessage {
            role: Role::Assistant,
            content: accumulated_text,
            tool_use_id: None,
            tool_calls: completed_tool_calls.clone(),
        };
        history.push(assistant_msg);
        let _ = working
            .conv_push(
                &session_id,
                ConvMessage {
                    role: "assistant".into(),
                    content: assistant_text,
                    tool_use_id: None,
                    ts: Utc::now(),
                },
            )
            .await;

        let latency_ms = start.elapsed().as_millis() as u64;
        let _ = state
            .broker
            .audit_memory_op(
                actor_id,
                ActionKind::Other,
                Some(&format!("llm://{}/{}", llm.provider(), llm.default_model())),
                Some(&format!(
                    "chat turn stop={:?} tools={}",
                    final_stop,
                    completed_tool_calls.len()
                )),
                latency_ms,
                !matches!(final_stop, StopReason::Error),
                vec![
                    ("session_id".into(), session_id.clone()),
                    ("input_tokens".into(), final_usage.input_tokens.to_string()),
                    ("output_tokens".into(), final_usage.output_tokens.to_string()),
                ],
            )
            .await;

        if !matches!(final_stop, StopReason::ToolUse) {
            // No tool use → conversation closes for this turn.
            send_ws(
                socket,
                WsServerMsg::AgentDone {
                    agent_id: "chat".into(),
                    session_id: session_id.clone(),
                    stop_reason: format!("{:?}", final_stop).to_lowercase(),
                    input_tokens: final_usage.input_tokens,
                    output_tokens: final_usage.output_tokens,
                },
            )
            .await;
            break;
        }

        // Dispatch each tool call, append results to history, loop again.
        for tc in &completed_tool_calls {
            let target_ref = tc
                .arguments
                .get("target_ref")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let net_req = NetworkRequest {
                capability: tc.name.clone(),
                target_ref: target_ref.clone(),
                timeout_secs: Some(30),
            };
            let result = network_agent.run(net_req).await;
            let (output, is_error) = match result {
                Ok(o) => {
                    let summary = format!(
                        "exit={} stdout_len={} stderr_len={}\n--- stdout ---\n{}\n--- stderr ---\n{}",
                        o.exit_status,
                        o.stdout.len(),
                        o.stderr.len(),
                        o.stdout,
                        o.stderr
                    );
                    (summary, o.exit_status != 0)
                }
                Err(e) => (format!("tool error: {e}"), true),
            };
            send_ws(
                socket,
                WsServerMsg::AgentToolResult {
                    agent_id: "chat".into(),
                    session_id: session_id.clone(),
                    tool: tc.name.clone(),
                    output: output.clone(),
                    is_error,
                },
            )
            .await;
            history.push(ChatMessage::tool_result(&tc.id, output.clone()));
            let _ = working
                .conv_push(
                    &session_id,
                    ConvMessage {
                        role: "tool".into(),
                        content: output,
                        tool_use_id: Some(tc.id.clone()),
                        ts: Utc::now(),
                    },
                )
                .await;
        }
    }

    info!(session = %session_id, "chat session updated");
}

fn conv_to_chat(c: ConvMessage) -> ChatMessage {
    let role = match c.role.as_str() {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::System,
    };
    ChatMessage {
        role,
        content: c.content,
        tool_use_id: c.tool_use_id,
        tool_calls: Vec::new(),
    }
}

struct PendingToolCall {
    id: String,
    name: String,
    input_json: String,
}

fn _unused_assistant_content(_: AssistantContent) {}

async fn send_ws(socket: &mut WebSocket, msg: WsServerMsg) {
    if let Ok(json) = serde_json::to_string(&msg) {
        let _ = socket.send(Message::Text(json.into())).await;
    }
}

async fn send_err(socket: &mut WebSocket, message: String) {
    let _ = socket
        .send(Message::Text(
            serde_json::to_string(&WsServerMsg::Error { message })
                .unwrap_or_default()
                .into(),
        ))
        .await;
}
