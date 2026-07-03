//! The `Gateway` abstraction (SDS §9.3 — FR-GW-04/05/06) and the canonical
//! inbound message it produces.
//!
//! A `Gateway` is a channel adapter presenting a uniform contract: verify an
//! inbound platform event, normalise it to an [`InboundMessage`], and address a
//! reply back to the originating conversation. Adding a channel means
//! implementing this trait and registering one route (webhook) or one supervised
//! task (poller) — no change to the chat turn loop, the Harness, or the Guard.
//!
//! ## Ingress split (P4 refinement over SDS §9.3)
//! The SDS shaped the trait around **webhook** channels (`verify_and_parse` on a
//! raw HTTP request). Matrix's natural ingress is a **`/sync` long-poll loop**,
//! not a webhook. So the abstraction distinguishes two ingress kinds:
//!
//! - **Webhook** (Telegram, Slack, WhatsApp) — the platform POSTs to an inbound
//!   route; the adapter implements [`Gateway::verify_and_parse`]. Authenticity is
//!   a per-request signature/secret.
//! - **Poller** (Matrix) — the adapter runs a long-lived [`PollingGateway::run_ingress`]
//!   task that authenticates its connection with a bot access token and pushes
//!   normalised messages into the shared pipeline. Authenticity is the token on
//!   the connection, not a per-request signature.
//!
//! Both converge on the identical downstream pipeline: bind identity →
//! `handle_chat_send` → reply via [`ReplySink`]. The trust properties
//! (FR-GW-07..11) hold for both — a poller's inbound is as authenticated as a
//! webhook's, and identity binding + guard/broker/audit are unchanged.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::reply_sink::ReplySink;

/// Stable channel identifier — `"telegram" | "matrix" | "slack" | ...`.
pub type ChannelId = String;

/// A verified, normalised inbound message from any channel.
#[derive(Debug, Clone)]
pub struct InboundMessage {
    /// Which adapter produced this (`self.channel()`).
    pub channel: ChannelId,
    /// The platform-side identity of the sender — a Telegram user id, a Matrix
    /// MXID (`@user:server`), etc. This is what identity binding resolves to an
    /// IAM user; it is NEVER trusted as an actor on its own (FR-GW-08).
    pub platform_handle: String,
    /// The message text the operator sent.
    pub text: String,
    /// How to address the reply back to the originating conversation.
    pub correlation: Correlation,
    /// When the adapter received/normalised the message.
    pub received_at: DateTime<Utc>,
}

/// Where a reply (or alert) is addressed on a channel. Carried on every
/// [`InboundMessage`] so the reply sink can address the response without the
/// Harness knowing the transport (FR-GW-12).
#[derive(Debug, Clone)]
pub struct Correlation {
    /// Thread / conversation container — a Telegram chat id, a Matrix room id.
    pub thread: Option<String>,
    /// The concrete destination the reply is posted to (chat id / room id).
    pub reply_to: String,
}

impl Correlation {
    /// A deterministic daimon session id for this conversation, so a given
    /// channel thread maps to one durable session across turns (SDS §9.4.3).
    pub fn session_id(&self, channel: &str) -> String {
        match &self.thread {
            Some(t) => format!("gw:{channel}:{t}"),
            None => format!("gw:{channel}:{}", self.reply_to),
        }
    }
}

/// A raw inbound HTTP request handed to a webhook adapter's `verify_and_parse`.
/// Header names are stored lowercased for case-insensitive lookup; the body is
/// the exact bytes received (signature verification is over the raw body).
pub struct InboundHttp {
    headers: std::collections::HashMap<String, String>,
    body: Vec<u8>,
}

impl InboundHttp {
    pub fn new(headers: std::collections::HashMap<String, String>, body: Vec<u8>) -> Self {
        let headers = headers
            .into_iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v))
            .collect();
        Self { headers, body }
    }

    /// Case-insensitive header lookup.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(|s| s.as_str())
    }

    /// The raw request body (verify signatures over this, not a re-serialisation).
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

/// Which ingress model an adapter uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ingress {
    /// The platform POSTs to an inbound route; verified per-request.
    Webhook,
    /// The adapter runs a long-lived poll loop (see [`PollingGateway`]).
    Poller,
}

/// Proactive-alert addressing (outbound). Defined here for the trait surface;
/// alert *routing* (FR-GW-13/14/15) is deferred to a later phase — P4 adapters
/// return `NotImplemented` from `deliver_alert`.
#[derive(Debug, Clone)]
pub struct Recipient {
    pub channel: ChannelId,
    pub to: String,
}

/// The body of a proactive alert.
#[derive(Debug, Clone)]
pub struct AlertBody {
    pub title: String,
    pub body: String,
}

impl AlertBody {
    /// Render to a single plain-text message (title, blank line, body). Both
    /// Telegram and Matrix send plain text, so this is the shared wire form.
    pub fn render(&self) -> String {
        if self.body.is_empty() {
            self.title.clone()
        } else {
            format!("{}\n\n{}", self.title, self.body)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    /// Verification failed — reject with HTTP 401, log, never reach the Harness.
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// Malformed / unparseable payload.
    #[error("bad request: {0}")]
    BadRequest(String),
    /// The message carried nothing actionable (e.g. a non-text update) — the
    /// adapter signals "ack and ignore" rather than an error.
    #[error("ignored: {0}")]
    Ignored(String),
    /// A downstream channel/transport error (outbound send, /sync fault).
    #[error("channel error: {0}")]
    Channel(String),
    /// Not wired in this phase (e.g. proactive alerts on a P4 adapter).
    #[error("not implemented")]
    NotImplemented,
}

/// The app-side inbound pipeline: bind identity → dispatch the turn → deliver the
/// reply. Implemented in `daimon-app` (it holds the IAM tables + the Harness);
/// both the webhook route and a poller adapter funnel every normalised message
/// through this single method, so there is exactly one authenticated dispatch
/// path (FR-GW-09). The adapter builds the `sink`; the handler binds identity
/// (fail-closed) and runs `handle_chat_send`.
#[async_trait]
pub trait InboundHandler: Send + Sync {
    async fn handle(&self, msg: InboundMessage, sink: Box<dyn ReplySink>);
}

/// A channel adapter. See the module docs for the ingress split.
#[async_trait]
pub trait Gateway: Send + Sync {
    /// This adapter's channel id.
    fn channel(&self) -> ChannelId;

    /// How this adapter receives inbound messages.
    fn ingress(&self) -> Ingress;

    /// Webhook adapters: verify authenticity (signature / secret token) and
    /// normalise a raw inbound HTTP request into an [`InboundMessage`]. Returns
    /// `Err` BEFORE anything reaches the Harness on verification failure
    /// (FR-GW-07). Poller adapters leave this as the default `NotImplemented`.
    async fn verify_and_parse(&self, _req: &InboundHttp) -> Result<InboundMessage, GatewayError> {
        Err(GatewayError::NotImplemented)
    }

    /// Build a [`ReplySink`] that addresses `correlation` on this channel.
    /// Streaming capability is per-adapter (Telegram/Matrix batch).
    fn reply_sink(&self, correlation: &Correlation) -> Box<dyn ReplySink>;

    /// Proactive outbound (alerts) to an explicit recipient. Alert routing is
    /// deferred to a later phase; P4 adapters return `NotImplemented`.
    async fn deliver_alert(
        &self,
        _to: &Recipient,
        _body: &AlertBody,
    ) -> Result<(), GatewayError> {
        Err(GatewayError::NotImplemented)
    }
}

/// A poller adapter additionally implements this to run its long-lived ingress
/// loop, delivering each normalised message to the shared `InboundHandler`. The
/// app supervises this as a background task (spawned only if the channel is
/// enabled). The loop owns its own authenticity (the bot access token on the
/// connection) and its own resume cursor.
#[async_trait]
pub trait PollingGateway: Gateway {
    /// Run until cancelled or a fatal error. Each inbound room/chat message is
    /// normalised and handed to `handler.handle(msg, self.reply_sink(&corr))`.
    async fn run_ingress(&self, handler: Arc<dyn InboundHandler>) -> Result<(), GatewayError>;
}

/// Persists a poller's resume cursor across restarts — Matrix's `/sync` `since`
/// token, Telegram's `getUpdates` offset. daimon-gateway has no DB access (D21),
/// so the store is injected; daimon-app backs it with `app_config`. A `None`
/// load means "cold start" (the adapter seeds a fresh cursor).
#[async_trait]
pub trait CursorStore: Send + Sync {
    async fn load(&self) -> Option<String>;
    async fn save(&self, cursor: &str);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_http_header_lookup_is_case_insensitive() {
        let mut h = std::collections::HashMap::new();
        h.insert("X-Telegram-Bot-Api-Secret-Token".to_string(), "tok".to_string());
        let req = InboundHttp::new(h, b"{}".to_vec());
        assert_eq!(req.header("x-telegram-bot-api-secret-token"), Some("tok"));
        assert_eq!(req.header("X-TELEGRAM-BOT-API-SECRET-TOKEN"), Some("tok"));
        assert_eq!(req.header("missing"), None);
        assert_eq!(req.body(), b"{}");
    }

    #[test]
    fn correlation_session_id_is_deterministic() {
        let c = Correlation {
            thread: Some("room!abc:hs".into()),
            reply_to: "room!abc:hs".into(),
        };
        assert_eq!(c.session_id("matrix"), "gw:matrix:room!abc:hs");
        // Same thread → same session across turns.
        assert_eq!(c.session_id("matrix"), c.session_id("matrix"));

        let no_thread = Correlation {
            thread: None,
            reply_to: "12345".into(),
        };
        assert_eq!(no_thread.session_id("telegram"), "gw:telegram:12345");
    }
}
