//! The transport-agnostic reply sink (SDS §9.2 — satisfies FR-GW-01/02/03).
//!
//! The chat turn loop (`daimon-app::chat::handle_chat_send`) used to write every
//! emission — token deltas, tool-use notices, tool results, done, errors —
//! straight to a concrete `axum::extract::ws::WebSocket`. That welded the loop
//! to the browser and left no seam for a messaging gateway to attach.
//!
//! This module defines the seam: a `TurnEvent` (one transport-neutral outbound
//! event of a turn) and a `ReplySink` (the delivery abstraction). The browser
//! WebSocket becomes *one* `ReplySink` among many (`WsSink`, in daimon-app); a
//! Telegram/Matrix adapter is another.
//!
//! **Streaming vs batched (FR-GW-02).** A streaming-capable transport (browser)
//! forwards each event as it arrives. A non-streaming transport (Telegram,
//! Matrix, e-mail) coalesces the token deltas into one message and flushes it on
//! `finish`. `BufferSink` implements that batched path once, generic over an
//! `OutboundChannel`, so each non-streaming adapter only implements a single
//! `send(text)` method rather than re-deriving the coalescing logic.
//!
//! **The sink abstracts delivery, never authority (FR-GW-03/10).** Swapping the
//! sink changes *where a reply is written*, not *what a turn is allowed to do*:
//! Harness dispatch, Guard gating, and the audit append are untouched, so a
//! browser turn and a gateway turn traverse the identical execution spine.

use async_trait::async_trait;

/// One outbound event of a chat turn, transport-neutral. These mirror exactly
/// the `WsServerMsg` variants the turn already emits, minus the socket.
#[derive(Debug, Clone)]
pub enum TurnEvent {
    /// A streamed token / content delta from the assistant.
    TokenDelta {
        session_id: String,
        content: String,
    },
    /// The LLM emitted a tool-use block; the server has dispatched it.
    ToolUse {
        session_id: String,
        tool: String,
        params: serde_json::Value,
    },
    /// A tool result fed back to the LLM (surfaced for the UI / channel).
    ToolResult {
        session_id: String,
        tool: String,
        output: String,
        is_error: bool,
    },
    /// The assistant turn completed — no more deltas until the next message.
    Done {
        session_id: String,
        stop_reason: String,
        input_tokens: u32,
        output_tokens: u32,
    },
    /// A turn-level error (LLM init/stream failure, etc.).
    Error { message: String },
}

/// Delivery abstraction. Streaming-capable transports (browser) forward each
/// event via `emit`; non-streaming transports buffer and flush on `finish`. The
/// turn loop calls `emit` per event and `finish` exactly once (FR-GW-02).
#[async_trait]
pub trait ReplySink: Send {
    /// Whether the transport wants incremental deltas. If false, a sink is
    /// expected to buffer `TokenDelta` text and emit nothing until `finish`.
    fn streams(&self) -> bool {
        true
    }

    /// Deliver one turn event.
    async fn emit(&mut self, event: TurnEvent);

    /// Called once at end of turn. For batched sinks, flush the coalesced reply.
    async fn finish(&mut self);
}

/// A channel that can post one coalesced message back to the originating
/// conversation. This is all a non-streaming adapter must implement to get the
/// batched `ReplySink` behaviour via `BufferSink` (FR-GW-02, FR-GW-12).
#[async_trait]
pub trait OutboundChannel: Send + Sync {
    /// Post one message. `Err` is logged and swallowed by the sink — reply
    /// delivery is best-effort and never stalls the turn (fail-soft, FR-GW-15).
    async fn send(&self, text: String) -> Result<(), String>;
}

/// The batched `ReplySink` for non-streaming channels. Accumulates `TokenDelta`
/// text (and appends any turn `Error`), then flushes a single message on
/// `finish` through its `OutboundChannel`. Tool-use / tool-result events are
/// dropped from the coalesced body by default — a chat-platform user wants the
/// answer, not the internal tool trace.
pub struct BufferSink<C: OutboundChannel> {
    channel: C,
    buf: String,
    had_error: bool,
}

impl<C: OutboundChannel> BufferSink<C> {
    pub fn new(channel: C) -> Self {
        Self {
            channel,
            buf: String::new(),
            had_error: false,
        }
    }
}

#[async_trait]
impl<C: OutboundChannel> ReplySink for BufferSink<C> {
    fn streams(&self) -> bool {
        false
    }

    async fn emit(&mut self, event: TurnEvent) {
        match event {
            TurnEvent::TokenDelta { content, .. } => self.buf.push_str(&content),
            TurnEvent::Error { message } => {
                self.had_error = true;
                if !self.buf.is_empty() {
                    self.buf.push('\n');
                }
                self.buf.push_str(&format!("\u{26a0}\u{fe0f} {message}"));
            }
            // Tool-use notices and tool results are internal trace — omitted from
            // the coalesced channel reply. (A future adapter may opt to render
            // them; the default keeps the channel message clean.)
            TurnEvent::ToolUse { .. } | TurnEvent::ToolResult { .. } | TurnEvent::Done { .. } => {}
        }
    }

    async fn finish(&mut self) {
        let text = std::mem::take(&mut self.buf);
        let text = if text.trim().is_empty() {
            if self.had_error {
                return; // an error with no body was already unrecoverable
            }
            // A turn that produced no assistant text (e.g. pure tool call with
            // nothing to say) still deserves an acknowledgement on a channel.
            "(no reply)".to_string()
        } else {
            text
        };
        if let Err(e) = self.channel.send(text).await {
            tracing::warn!(error = %e, "gateway reply flush failed (fail-soft)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A test channel that records what got flushed.
    #[derive(Clone, Default)]
    struct RecordingChannel {
        sent: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl OutboundChannel for RecordingChannel {
        async fn send(&self, text: String) -> Result<(), String> {
            self.sent.lock().unwrap().push(text);
            Ok(())
        }
    }

    #[tokio::test]
    async fn buffer_sink_coalesces_deltas_into_one_message() {
        // AC-P4-02 — the non-streaming path: many TokenDeltas → one flushed
        // message on finish, tool events omitted.
        let chan = RecordingChannel::default();
        let mut sink = BufferSink::new(chan.clone());
        assert!(!sink.streams());

        sink.emit(TurnEvent::TokenDelta {
            session_id: "s".into(),
            content: "Hello".into(),
        })
        .await;
        sink.emit(TurnEvent::ToolUse {
            session_id: "s".into(),
            tool: "network.routeros.firewall_list".into(),
            params: serde_json::json!({}),
        })
        .await;
        sink.emit(TurnEvent::TokenDelta {
            session_id: "s".into(),
            content: ", world".into(),
        })
        .await;
        sink.emit(TurnEvent::Done {
            session_id: "s".into(),
            stop_reason: "end_turn".into(),
            input_tokens: 10,
            output_tokens: 3,
        })
        .await;
        sink.finish().await;

        let sent = chan.sent.lock().unwrap();
        assert_eq!(sent.len(), 1, "exactly one coalesced flush");
        assert_eq!(sent[0], "Hello, world");
    }

    #[tokio::test]
    async fn buffer_sink_appends_turn_error() {
        let chan = RecordingChannel::default();
        let mut sink = BufferSink::new(chan.clone());
        sink.emit(TurnEvent::TokenDelta {
            session_id: "s".into(),
            content: "partial".into(),
        })
        .await;
        sink.emit(TurnEvent::Error {
            message: "llm stream: boom".into(),
        })
        .await;
        sink.finish().await;
        let sent = chan.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("partial"));
        assert!(sent[0].contains("llm stream: boom"));
    }

    #[tokio::test]
    async fn buffer_sink_empty_turn_acks() {
        let chan = RecordingChannel::default();
        let mut sink = BufferSink::new(chan.clone());
        sink.emit(TurnEvent::Done {
            session_id: "s".into(),
            stop_reason: "end_turn".into(),
            input_tokens: 1,
            output_tokens: 0,
        })
        .await;
        sink.finish().await;
        let sent = chan.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], "(no reply)");
    }
}
