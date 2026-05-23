//! Phase 4 D3 — /chat operator UI.
//!
//! Connects to `/api/v1/ws`, sends `ChatSend` on submit, and renders the
//! streamed `AgentTokenDelta` / `AgentToolUse` / `AgentToolResult` /
//! `AgentDone` messages into a conversation log.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// A turn in the rendered transcript.
#[derive(Debug, Clone)]
struct Turn {
    pub kind: TurnKind,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TurnKind {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Debug, Clone, Serialize)]
struct ChatSendMsg<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    session_id: String,
    user_message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
enum IncomingMsg {
    AgentTokenDelta {
        agent_id: String,
        session_id: String,
        content_delta: String,
    },
    AgentToolUse {
        agent_id: String,
        session_id: String,
        tool: String,
        params: serde_json::Value,
    },
    AgentToolResult {
        agent_id: String,
        session_id: String,
        tool: String,
        output: String,
        is_error: bool,
    },
    AgentDone {
        agent_id: String,
        session_id: String,
        stop_reason: String,
        input_tokens: u32,
        output_tokens: u32,
    },
    Error {
        message: String,
    },
    #[serde(other)]
    Other,
}

#[component]
pub fn Chat() -> impl IntoView {
    let (turns, set_turns) = signal::<Vec<Turn>>(Vec::new());
    let (input, set_input) = signal(String::new());
    let (status, set_status) = signal::<String>("connecting...".into());
    let (busy, set_busy) = signal(false);
    let session_id = StoredValue::new(format!("sess-{}", random_suffix()));

    // Stable handle to the WebSocket — we open once on mount, drop on unmount.
    #[cfg(feature = "hydrate")]
    let socket = StoredValue::new(Option::<web_sys::WebSocket>::None);

    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        use wasm_bindgen::JsCast;
        use wasm_bindgen::prelude::Closure;
        use web_sys::{MessageEvent, WebSocket};

        let Some(window) = web_sys::window() else { return };
        let location = window.location();
        let proto = if location.protocol().unwrap_or_default() == "https:" {
            "wss"
        } else {
            "ws"
        };
        let host = location.host().unwrap_or_default();
        let url = format!("{proto}://{host}/api/v1/ws");

        let ws = match WebSocket::new(&url) {
            Ok(ws) => ws,
            Err(_) => {
                set_status.set("ws open failed".into());
                return;
            }
        };

        let on_open_status = set_status;
        let on_open = Closure::<dyn FnMut(_)>::new(move |_e: web_sys::Event| {
            on_open_status.set("connected".into());
        });
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        on_open.forget();

        let on_message_turns = set_turns;
        let on_message_busy = set_busy;
        let on_message_status = set_status;
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            let Ok(text) = e.data().dyn_into::<js_sys::JsString>() else { return };
            let text: String = text.into();
            let Ok(msg) = serde_json::from_str::<IncomingMsg>(&text) else { return };
            match msg {
                IncomingMsg::AgentTokenDelta { content_delta, .. } => {
                    on_message_turns.update(|ts| {
                        // Append to the latest assistant turn — create one if the
                        // newest turn isn't an assistant block (handles the case
                        // after a tool round-trip).
                        if matches!(ts.last(), Some(t) if t.kind == TurnKind::Assistant) {
                            if let Some(last) = ts.last_mut() {
                                last.content.push_str(&content_delta);
                            }
                        } else {
                            ts.push(Turn {
                                kind: TurnKind::Assistant,
                                content: content_delta,
                            });
                        }
                    });
                }
                IncomingMsg::AgentToolUse { tool, params, .. } => {
                    let line = format!("calling {tool} with {}", params);
                    on_message_turns.update(|ts| ts.push(Turn { kind: TurnKind::Tool, content: line }));
                }
                IncomingMsg::AgentToolResult {
                    tool, output, is_error, ..
                } => {
                    let prefix = if is_error { "✗" } else { "✓" };
                    let body = if output.len() > 1200 {
                        format!("{}…", &output[..1200])
                    } else {
                        output
                    };
                    let line = format!("{prefix} {tool}\n{body}");
                    on_message_turns.update(|ts| ts.push(Turn { kind: TurnKind::Tool, content: line }));
                }
                IncomingMsg::AgentDone {
                    stop_reason,
                    input_tokens,
                    output_tokens,
                    ..
                } => {
                    on_message_status.set(format!(
                        "done (stop={stop_reason}, in={input_tokens}, out={output_tokens})"
                    ));
                    on_message_busy.set(false);
                }
                IncomingMsg::Error { message } => {
                    on_message_turns.update(|ts| {
                        ts.push(Turn {
                            kind: TurnKind::System,
                            content: format!("error: {message}"),
                        })
                    });
                    on_message_busy.set(false);
                }
                IncomingMsg::Other => {}
            }
        });
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();

        let on_close_status = set_status;
        let on_close = Closure::<dyn FnMut(_)>::new(move |_e: web_sys::CloseEvent| {
            on_close_status.set("disconnected".into());
        });
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        on_close.forget();

        socket.set_value(Some(ws));
    });

    let send_message = move |_| {
        let text = input.get_untracked().trim().to_string();
        if text.is_empty() {
            return;
        }
        set_input.set(String::new());
        set_busy.set(true);

        set_turns.update(|ts| {
            ts.push(Turn {
                kind: TurnKind::User,
                content: text.clone(),
            });
            ts.push(Turn {
                kind: TurnKind::Assistant,
                content: String::new(),
            });
        });

        #[cfg(feature = "hydrate")]
        {
            let Some(ws) = socket.get_value() else {
                set_busy.set(false);
                return;
            };
            let payload = ChatSendMsg {
                ty: "ChatSend",
                session_id: session_id.get_value(),
                user_message: text,
                model: None,
            };
            let json = serde_json::to_string(&payload).unwrap_or_default();
            let _ = ws.send_with_str(&json);
        }
    };

    let key_handler = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Enter" && !ev.shift_key() {
            ev.prevent_default();
            send_message(());
        }
    };

    view! {
        <div class="flex flex-col h-full">
            <div class="px-4 py-3 border-b border-border-primary flex items-center justify-between">
                <h1 class="text-lg font-semibold text-text-primary">"Chat"</h1>
                <div class="text-xs text-text-secondary font-mono">{move || status.get()}</div>
            </div>
            <div class="flex-1 overflow-y-auto px-4 py-4 space-y-3 bg-surface-primary">
                {move || turns.get().into_iter().enumerate().map(|(i, t)| view! {
                    <TurnBubble idx=i turn=t />
                }).collect_view()}
            </div>
            <div class="border-t border-border-primary px-4 py-3 bg-surface-secondary">
                <div class="flex gap-2">
                    <textarea
                        class="flex-1 px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber resize-none"
                        rows="2"
                        placeholder="Ask dAImon to read a device, list interfaces, etc. (Enter to send, Shift+Enter for newline)"
                        prop:value=move || input.get()
                        on:input=move |ev| set_input.set(event_target_value(&ev))
                        on:keydown=key_handler
                    />
                    <button
                        on:click=move |_| send_message(())
                        class="px-4 py-2 bg-accent-amber text-surface-primary font-medium rounded-md hover:bg-accent-amber/90 transition-colors text-sm disabled:opacity-50"
                        disabled=move || busy.get()
                    >
                        {move || if busy.get() { "..." } else { "Send" }}
                    </button>
                </div>
            </div>
        </div>
    }
}

#[component]
fn TurnBubble(idx: usize, turn: Turn) -> impl IntoView {
    let _ = idx;
    let (border, accent, label) = match turn.kind {
        TurnKind::User => ("border-blue-500/30", "text-blue-300", "you"),
        TurnKind::Assistant => ("border-accent-amber/30", "text-accent-amber", "daimon"),
        TurnKind::Tool => ("border-purple-500/30", "text-purple-300", "tool"),
        TurnKind::System => ("border-accent-danger/30", "text-accent-danger", "system"),
    };
    view! {
        <div class=format!("rounded-md border {} bg-surface-secondary p-3", border)>
            <div class=format!("text-[10px] font-mono uppercase tracking-wider mb-1 {}", accent)>
                {label}
            </div>
            <pre class="text-sm text-text-primary whitespace-pre-wrap font-sans">{turn.content}</pre>
        </div>
    }
}

#[cfg(feature = "hydrate")]
fn random_suffix() -> String {
    use wasm_bindgen::JsCast;
    let win = web_sys::window().expect("window");
    let crypto = win.crypto().expect("crypto");
    let mut buf = [0u8; 6];
    let _ = crypto.get_random_values_with_u8_array(&mut buf);
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(not(feature = "hydrate"))]
fn random_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t:x}")
}
