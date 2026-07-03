//! Phase 4 D3 (revised v2) — floating ChatBubble with multi-session support.
//!
//! Mounted in `Layout` above `<Outlet />` so the WebSocket + UI state
//! persist across navigation.
//!
//! Features:
//! - Floating amber circular button bottom-right.
//! - Expanded panel is resizable (CSS `resize: both`).
//! - Session list in the header — collapsible dropdown showing all known
//!   sessions for this operator (persisted to `localStorage`). Each entry
//!   has a delete button.
//! - "+ New chat" button creates a fresh `session_id` and switches active.
//! - Switching session calls `load_chat_session` to fetch existing history
//!   from Redis (working memory) and repopulates the turns UI.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct SessionMeta {
    pub id: String,
    pub title: String,
    pub created_at: i64,
}

const LS_KEY: &str = "daimon.chat.sessions";
const LS_CURRENT: &str = "daimon.chat.current";

#[derive(Debug, Clone, Serialize)]
struct ChatSendMsg<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    session_id: String,
    user_message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    effort: Option<String>,
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
pub fn ChatBubble() -> impl IntoView {
    let (open, set_open) = signal(false);
    let (sessions_open, set_sessions_open) = signal(false);
    let (turns, set_turns) = signal::<Vec<Turn>>(Vec::new());
    let (input, set_input) = signal(String::new());
    let (status, set_status) = signal::<String>("connecting...".into());
    let (busy, set_busy) = signal(false);

    let sessions = RwSignal::new(Vec::<SessionMeta>::new());
    let current_session = RwSignal::new(String::new());

    // P7-7 (FR-UI-05/15/16): the operator's model/effort selection (empty = use
    // the server default). The offered models come from the server's permitted
    // set — the SAME set it enforces — so a pick is always honourable.
    let selected_model = RwSignal::new(String::new());
    let selected_effort = RwSignal::new(String::new());
    let available_models = RwSignal::new(Vec::<String>::new());

    #[cfg(feature = "hydrate")]
    let socket = StoredValue::new(Option::<web_sys::WebSocket>::None);

    // ---- Boot: load sessions from localStorage, pick current ----
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
            let loaded: Vec<SessionMeta> = storage
                .get_item(LS_KEY)
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let current = storage.get_item(LS_CURRENT).ok().flatten().unwrap_or_default();
            if loaded.is_empty() {
                let fresh = SessionMeta {
                    id: format!("sess-{}", random_suffix()),
                    title: "New chat".into(),
                    created_at: now_epoch(),
                };
                current_session.set(fresh.id.clone());
                sessions.set(vec![fresh]);
            } else {
                let pick = if !current.is_empty() && loaded.iter().any(|s| s.id == current) {
                    current
                } else {
                    loaded[0].id.clone()
                };
                current_session.set(pick);
                sessions.set(loaded);
            }
        } else {
            let fresh = SessionMeta {
                id: format!("sess-{}", random_suffix()),
                title: "New chat".into(),
                created_at: now_epoch(),
            };
            current_session.set(fresh.id.clone());
            sessions.set(vec![fresh]);
        }
    });

    // ---- P7-5 (FR-UI-19): source the durable, owner-scoped session list from
    // the server so it follows the user across browsers. When the server has
    // sessions, it is authoritative over the localStorage cache. ----
    let refresh_sessions = Action::new(move |_: &()| async move {
        crate::admin_chat_sessions::list_my_sessions().await
    });
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        // dispatch once on mount (no tracked deps → runs a single time)
        refresh_sessions.dispatch(());
    });
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if let Some(Ok(server)) = refresh_sessions.value().get() {
            if !server.is_empty() {
                let mapped: Vec<SessionMeta> = server
                    .into_iter()
                    .map(|s| SessionMeta {
                        id: s.id,
                        title: s.title,
                        created_at: 0,
                    })
                    .collect();
                if !mapped.iter().any(|m| m.id == current_session.get_untracked()) {
                    if let Some(first) = mapped.first() {
                        current_session.set(first.id.clone());
                    }
                }
                sessions.set(mapped);
            }
        }
    });

    // ---- P7-7: fetch the permitted model set for the picker ----
    let refresh_models = Action::new(move |_: &()| async move {
        crate::admin_chat_sessions::list_available_models().await
    });
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        refresh_models.dispatch(());
    });
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if let Some(Ok(m)) = refresh_models.value().get() {
            available_models.set(m);
        }
    });

    // ---- Persist on changes ----
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        let s = sessions.get();
        let c = current_session.get();
        if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
            if let Ok(json) = serde_json::to_string(&s) {
                let _ = storage.set_item(LS_KEY, &json);
            }
            let _ = storage.set_item(LS_CURRENT, &c);
        }
    });

    // ---- WebSocket open ----
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
            on_open_status.set("ready".into());
        });
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        on_open.forget();

        let on_message_turns = set_turns;
        let on_message_turns_read = turns;
        let on_message_busy = set_busy;
        let on_message_status = set_status;
        let on_message_sessions = sessions;
        let on_message_current = current_session;
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            let Ok(text) = e.data().dyn_into::<js_sys::JsString>() else { return };
            let text: String = text.into();
            let Ok(msg) = serde_json::from_str::<IncomingMsg>(&text) else { return };
            match msg {
                IncomingMsg::AgentTokenDelta { content_delta, .. } => {
                    on_message_turns.update(|ts| {
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
                IncomingMsg::AgentToolResult { tool, output, is_error, .. } => {
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
                    input_tokens,
                    output_tokens,
                    stop_reason,
                    ..
                } => {
                    on_message_status.set(format!("done · {} in / {} out", input_tokens, output_tokens));
                    on_message_busy.set(false);
                    let _ = stop_reason;
                    let cur = on_message_current.get_untracked();
                    on_message_sessions.update(|list| {
                        if let Some(s) = list.iter_mut().find(|s| s.id == cur) {
                            if s.title == "New chat" {
                                if let Some(first_user) = on_message_turns_read
                                    .get_untracked()
                                    .iter()
                                    .find(|t| t.kind == TurnKind::User)
                                {
                                    s.title = truncate_title(&first_user.content);
                                }
                            }
                        }
                    });
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

    // ---- Switch session (load history from Redis) ----
    let switch_session_action = Action::new(move |id: &String| {
        let id = id.clone();
        async move {
            use crate::admin_chat_sessions::load_chat_session;
            current_session.set(id.clone());
            set_turns.set(Vec::new());
            set_sessions_open.set(false);
            match load_chat_session(id).await {
                Ok(history) => {
                    let loaded: Vec<Turn> = history
                        .into_iter()
                        .map(|t| Turn {
                            kind: match t.role.as_str() {
                                "user" => TurnKind::User,
                                "assistant" => TurnKind::Assistant,
                                "tool" => TurnKind::Tool,
                                _ => TurnKind::System,
                            },
                            content: t.content,
                        })
                        .collect();
                    set_turns.set(loaded);
                }
                Err(e) => {
                    set_turns.update(|ts| {
                        ts.push(Turn {
                            kind: TurnKind::System,
                            content: format!("load history error: {e}"),
                        })
                    });
                }
            }
        }
    });

    let new_chat = move |_| {
        let id = format!("sess-{}", random_suffix());
        let fresh = SessionMeta {
            id: id.clone(),
            title: "New chat".into(),
            created_at: now_epoch(),
        };
        sessions.update(|list| list.insert(0, fresh));
        current_session.set(id);
        set_turns.set(Vec::new());
        set_sessions_open.set(false);
    };

    let delete_session_action = Action::new(move |id: &String| {
        let id = id.clone();
        async move {
            use crate::admin_chat_sessions::delete_chat_session;
            let _ = delete_chat_session(id.clone()).await;
            sessions.update(|list| list.retain(|s| s.id != id));
            let cur = current_session.get_untracked();
            if cur == id {
                let next = sessions.get_untracked().first().cloned();
                if let Some(s) = next {
                    current_session.set(s.id);
                } else {
                    let fresh = SessionMeta {
                        id: format!("sess-{}", random_suffix()),
                        title: "New chat".into(),
                        created_at: now_epoch(),
                    };
                    current_session.set(fresh.id.clone());
                    sessions.set(vec![fresh]);
                }
                set_turns.set(Vec::new());
            }
        }
    });

    let send_message = move |_| {
        let text = input.get_untracked().trim().to_string();
        if text.is_empty() {
            return;
        }
        set_input.set(String::new());
        set_busy.set(true);

        set_turns.update(|ts| {
            ts.push(Turn { kind: TurnKind::User, content: text.clone() });
            ts.push(Turn { kind: TurnKind::Assistant, content: String::new() });
        });

        #[cfg(feature = "hydrate")]
        {
            let Some(ws) = socket.get_value() else {
                set_busy.set(false);
                return;
            };
            let pick = |v: String| if v.is_empty() { None } else { Some(v) };
            let payload = ChatSendMsg {
                ty: "ChatSend",
                session_id: current_session.get_untracked(),
                user_message: text,
                model: pick(selected_model.get_untracked()),
                effort: pick(selected_effort.get_untracked()),
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

    let current_title = move || {
        let cur = current_session.get();
        sessions
            .get()
            .into_iter()
            .find(|s| s.id == cur)
            .map(|s| s.title)
            .unwrap_or_else(|| "New chat".into())
    };

    view! {
        <button
            on:click=move |_| set_open.update(|o| *o = !*o)
            class=move || format!(
                "fixed bottom-6 right-6 z-40 w-12 h-12 rounded-full bg-accent-amber text-surface-primary shadow-lg hover:scale-105 transition-transform flex items-center justify-center {}",
                if open.get() { "rotate-45" } else { "" }
            )
            aria-label="Chat"
        >
            <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" />
            </svg>
        </button>

        <Show when=move || open.get()>
            <div
                class="fixed bottom-24 right-6 z-40 flex flex-col bg-surface-secondary border border-border-primary rounded-lg shadow-2xl overflow-hidden"
                style="width: 420px; height: 580px; min-width: 320px; min-height: 360px; max-width: calc(100vw - 3rem); max-height: calc(100vh - 7rem); resize: both;"
            >
                <div class="px-3 py-2 border-b border-border-primary flex items-center gap-2 bg-surface-tertiary/40 shrink-0">
                    <button
                        on:click=move |_| set_sessions_open.update(|o| *o = !*o)
                        class="flex-1 min-w-0 flex items-center gap-2 text-left px-2 py-1 rounded hover:bg-surface-tertiary"
                    >
                        <svg
                            class=move || format!(
                                "w-3 h-3 text-text-muted transition-transform {}",
                                if sessions_open.get() { "rotate-180" } else { "" }
                            )
                            fill="none" stroke="currentColor" viewBox="0 0 24 24"
                        >
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                d="M19 9l-7 7-7-7" />
                        </svg>
                        <div class="min-w-0 flex-1">
                            <div class="text-sm font-semibold text-text-primary truncate">{current_title}</div>
                            <div class="text-[10px] font-mono text-text-muted">{move || status.get()}</div>
                        </div>
                    </button>
                    <button
                        on:click=new_chat
                        class="w-7 h-7 flex items-center justify-center rounded-md text-text-muted hover:text-text-primary hover:bg-surface-tertiary"
                        aria-label="New chat"
                        title="New chat"
                    >
                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
                        </svg>
                    </button>
                    <button
                        on:click=move |_| set_open.set(false)
                        class="w-7 h-7 flex items-center justify-center rounded-md text-text-muted hover:text-text-primary hover:bg-surface-tertiary"
                        aria-label="Close"
                    >
                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
                        </svg>
                    </button>
                </div>

                <Show when=move || sessions_open.get()>
                    <div class="border-b border-border-primary max-h-56 overflow-y-auto bg-surface-primary">
                        {move || sessions.get().into_iter().map(|s| {
                            let id_click = s.id.clone();
                            let id_del = s.id.clone();
                            let is_current = Memo::new({
                                let id = s.id.clone();
                                move |_| current_session.get() == id
                            });
                            view! {
                                <div class="flex items-center group hover:bg-surface-secondary">
                                    <button
                                        on:click=move |_| {
                                            switch_session_action.dispatch(id_click.clone());
                                        }
                                        class=move || format!(
                                            "flex-1 text-left px-3 py-2 truncate text-sm {}",
                                            if is_current.get() { "text-accent-amber" } else { "text-text-primary" }
                                        )
                                    >
                                        {s.title}
                                    </button>
                                    <button
                                        on:click=move |ev| {
                                            ev.stop_propagation();
                                            delete_session_action.dispatch(id_del.clone());
                                        }
                                        class="px-2 py-2 text-text-muted hover:text-accent-danger opacity-0 group-hover:opacity-100 transition-opacity"
                                        aria-label="Delete session"
                                    >
                                        <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6M1 7h22M9 7V4a2 2 0 012-2h2a2 2 0 012 2v3" />
                                        </svg>
                                    </button>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                </Show>

                <div class="flex-1 overflow-y-auto px-3 py-3 space-y-2 bg-surface-primary">
                    {move || {
                        let ts = turns.get();
                        if ts.is_empty() {
                            view! {
                                <div class="text-text-muted text-sm text-center mt-8 px-6">
                                    "Ask me to read a device, list interfaces, or query the cluster."
                                </div>
                            }.into_any()
                        } else {
                            ts.into_iter().enumerate().map(|(i, t)| view! {
                                <TurnBubble idx=i turn=t />
                            }).collect_view().into_any()
                        }
                    }}
                </div>

                <div class="border-t border-border-primary px-3 py-2 bg-surface-secondary shrink-0">
                    // P7-7: model + effort picker (server-permitted set; unpermitted
                    // picks are rejected server-side, never silently substituted).
                    <div class="flex gap-2 mb-2">
                        <Show when=move || !available_models.get().is_empty()>
                            <select
                                class="flex-1 min-w-0 px-2 py-1 bg-surface-tertiary border border-border-primary rounded text-text-secondary text-[11px] font-mono"
                                on:change=move |ev| selected_model.set(event_target_value(&ev))
                                prop:value=move || selected_model.get()
                            >
                                <option value="">"model: default"</option>
                                {move || available_models.get().into_iter().map(|m| {
                                    let label = m.clone();
                                    view! { <option value=m>{label}</option> }
                                }).collect_view()}
                            </select>
                        </Show>
                        <select
                            class="px-2 py-1 bg-surface-tertiary border border-border-primary rounded text-text-secondary text-[11px] font-mono"
                            on:change=move |ev| selected_effort.set(event_target_value(&ev))
                            prop:value=move || selected_effort.get()
                        >
                            <option value="">"effort: default"</option>
                            <option value="fast">"fast"</option>
                            <option value="deliberate">"deliberate"</option>
                        </select>
                    </div>
                    <div class="flex gap-2">
                        <textarea
                            class="flex-1 px-2 py-1.5 bg-surface-tertiary border border-border-primary rounded text-text-primary text-sm focus:outline-none focus:border-accent-amber resize-none"
                            rows="2"
                            placeholder="Enter to send · Shift+Enter for newline"
                            prop:value=move || input.get()
                            on:input=move |ev| set_input.set(event_target_value(&ev))
                            on:keydown=key_handler
                        />
                        <button
                            on:click=move |_| send_message(())
                            class="px-3 py-1 bg-accent-amber text-surface-primary font-medium rounded text-sm disabled:opacity-50"
                            disabled=busy
                        >
                            {move || if busy.get() { "…" } else { "Send" }}
                        </button>
                    </div>
                </div>
            </div>
        </Show>
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
        <div class=format!("rounded-md border {} bg-surface-secondary p-2", border)>
            <div class=format!("text-[9px] font-mono uppercase tracking-wider mb-1 {}", accent)>
                {label}
            </div>
            <pre class="text-[13px] text-text-primary whitespace-pre-wrap font-sans">{turn.content}</pre>
        </div>
    }
}

fn truncate_title(s: &str) -> String {
    let cleaned: String = s.lines().next().unwrap_or("").chars().take(48).collect();
    if cleaned.is_empty() {
        "New chat".into()
    } else {
        cleaned
    }
}

#[cfg(feature = "hydrate")]
fn random_suffix() -> String {
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

#[cfg(feature = "hydrate")]
fn now_epoch() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

#[cfg(not(feature = "hydrate"))]
fn now_epoch() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
