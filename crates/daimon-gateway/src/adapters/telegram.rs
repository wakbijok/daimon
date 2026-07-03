//! Telegram webhook adapter (SDS §9.3/§9.4 — FR-GW-05/07/12).
//!
//! Ingress is a **webhook**: Telegram POSTs updates to daimon's inbound route,
//! authenticated by a shared secret echoed in `X-Telegram-Bot-Api-Secret-Token`
//! (registered with the bot at `setWebhook`). The adapter verifies that secret
//! constant-time BEFORE parsing (FR-GW-07), normalises the update to an
//! [`InboundMessage`], and addresses replies back to the originating chat via a
//! batched [`BufferSink`] over `sendMessage` (FR-GW-12).
//!
//! The bot token is daimon's own credential (like the LLM API key), resolved
//! from the vault by reference at boot and passed in by value here — never
//! logged (FR-GW-17).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

use crate::gateway::{
    AlertBody, ChannelId, Correlation, CursorStore, Gateway, GatewayError, InboundHandler,
    InboundHttp, InboundMessage, Ingress, PollingGateway, Recipient,
};
use crate::reply_sink::{BufferSink, OutboundChannel, ReplySink};
use crate::verify;

const DEFAULT_API_BASE: &str = "https://api.telegram.org";
const SECRET_HEADER: &str = "x-telegram-bot-api-secret-token";
const POLL_TIMEOUT_SECS: u64 = 30;
const POLL_BACKOFF: Duration = Duration::from_secs(5);

/// A Telegram bot as a daimon `Gateway`.
pub struct TelegramAdapter {
    bot_token: String,
    webhook_secret: String,
    api_base: String,
    http: reqwest::Client,
}

impl TelegramAdapter {
    /// Production constructor — talks to `https://api.telegram.org`.
    pub fn new(bot_token: String, webhook_secret: String) -> Self {
        Self::with_api_base(bot_token, webhook_secret, DEFAULT_API_BASE.to_string())
    }

    /// Test / self-hosted-proxy constructor with an overridable API base.
    pub fn with_api_base(bot_token: String, webhook_secret: String, api_base: String) -> Self {
        Self {
            bot_token,
            webhook_secret,
            api_base: api_base.trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
        }
    }
}

// ---- Telegram Update wire types (partial — only the fields we consume) ----

#[derive(Deserialize)]
struct Update {
    #[serde(default)]
    update_id: i64,
    #[serde(default)]
    message: Option<TgMessage>,
}

#[derive(Deserialize)]
struct TgMessage {
    #[serde(default)]
    text: Option<String>,
    from: Option<TgUser>,
    chat: TgChat,
}

#[derive(Deserialize)]
struct TgUser {
    id: i64,
}

#[derive(Deserialize)]
struct TgChat {
    id: i64,
}

/// `getUpdates` response envelope (poller ingress).
#[derive(Deserialize)]
struct GetUpdates {
    #[serde(default)]
    result: Vec<Update>,
}

/// Normalise a Telegram `Update` to a canonical `InboundMessage`. Shared by the
/// webhook adapter (`verify_and_parse`) and the poll adapter (`getUpdates`).
/// Bind on the sender's numeric id; reply to the chat the message arrived in.
/// A non-text / senderless update is `Ignored` (ack + skip, never an error).
fn update_to_inbound(update: Update) -> Result<InboundMessage, GatewayError> {
    let message = update
        .message
        .ok_or_else(|| GatewayError::Ignored("update has no message".into()))?;
    let text = message
        .text
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| GatewayError::Ignored("message has no text".into()))?;
    let from = message
        .from
        .ok_or_else(|| GatewayError::Ignored("message has no sender".into()))?;
    let chat_id = message.chat.id.to_string();
    Ok(InboundMessage {
        channel: "telegram".into(),
        platform_handle: from.id.to_string(),
        text,
        correlation: Correlation {
            thread: Some(chat_id.clone()),
            reply_to: chat_id,
        },
        received_at: Utc::now(),
    })
}

/// P6-9 (FR-GW-13): post a proactive alert to an explicit chat via `sendMessage`.
/// Shared by both Telegram adapters' `deliver_alert`. The bot token rides in the
/// URL path (Telegram's API design), so `without_url()` strips it from any
/// transport-error Display before it can reach a log (FR-GW-17).
async fn telegram_send_alert(
    http: &reqwest::Client,
    api_base: &str,
    bot_token: &str,
    chat_id: &str,
    text: &str,
) -> Result<(), GatewayError> {
    let url = format!("{api_base}/bot{bot_token}/sendMessage");
    let resp = http
        .post(&url)
        .json(&serde_json::json!({ "chat_id": chat_id, "text": text }))
        .send()
        .await
        .map_err(|e| GatewayError::Channel(format!("telegram deliver_alert: {}", e.without_url())))?;
    if !resp.status().is_success() {
        return Err(GatewayError::Channel(format!(
            "telegram deliver_alert HTTP {}",
            resp.status()
        )));
    }
    Ok(())
}

/// Build the batched reply sink that posts one `sendMessage` per turn to
/// `reply_to`. Shared by both the webhook and poll adapters.
fn telegram_sink(
    http: reqwest::Client,
    api_base: String,
    bot_token: String,
    reply_to: String,
) -> Box<dyn ReplySink> {
    Box::new(BufferSink::new(TelegramOutbound {
        http,
        api_base,
        bot_token,
        chat_id: reply_to,
    }))
}

#[async_trait]
impl Gateway for TelegramAdapter {
    fn channel(&self) -> ChannelId {
        "telegram".into()
    }

    fn ingress(&self) -> Ingress {
        Ingress::Webhook
    }

    async fn deliver_alert(&self, to: &Recipient, body: &AlertBody) -> Result<(), GatewayError> {
        telegram_send_alert(&self.http, &self.api_base, &self.bot_token, &to.to, &body.render()).await
    }

    async fn verify_and_parse(&self, req: &InboundHttp) -> Result<InboundMessage, GatewayError> {
        // FR-GW-07: authenticity FIRST — a wrong/absent secret token is rejected
        // before the body is even parsed, so nothing reaches the Harness/LLM.
        if !verify::verify_secret_token(&self.webhook_secret, req.header(SECRET_HEADER)) {
            return Err(GatewayError::Unauthorized(
                "telegram secret token mismatch".into(),
            ));
        }
        let update: Update = serde_json::from_slice(req.body())
            .map_err(|e| GatewayError::BadRequest(format!("telegram update json: {e}")))?;
        // A webhook fires for many update kinds (edits, joins, callbacks). Only a
        // text message dispatches; everything else is acked-and-ignored.
        update_to_inbound(update)
    }

    fn reply_sink(&self, correlation: &Correlation) -> Box<dyn ReplySink> {
        telegram_sink(
            self.http.clone(),
            self.api_base.clone(),
            self.bot_token.clone(),
            correlation.reply_to.clone(),
        )
    }
}

/// A Telegram bot as a **poller** `Gateway` (SDS §9.8 poller ingress). Instead of
/// a public webhook, it long-polls `getUpdates` — no ingress, no `setWebhook`,
/// runs anywhere (this is how daimon reuses an internal bot). Authenticity is the
/// bot token on the connection; the update offset is persisted via [`CursorStore`]
/// so a restart does not reprocess. `verify_and_parse` stays `NotImplemented`.
pub struct TelegramPollAdapter {
    bot_token: String,
    api_base: String,
    http: reqwest::Client,
    cursor: Arc<dyn CursorStore>,
}

impl TelegramPollAdapter {
    pub fn new(bot_token: String, cursor: Arc<dyn CursorStore>) -> Self {
        Self::with_api_base(bot_token, DEFAULT_API_BASE.to_string(), cursor)
    }

    pub fn with_api_base(
        bot_token: String,
        api_base: String,
        cursor: Arc<dyn CursorStore>,
    ) -> Self {
        // A client timeout slightly above the long-poll window so a hung
        // connection cannot stall the loop forever.
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(POLL_TIMEOUT_SECS + 5))
            .build()
            .unwrap_or_default();
        Self {
            bot_token,
            api_base: api_base.trim_end_matches('/').to_string(),
            http,
            cursor,
        }
    }

    /// `getUpdates` fails (409) while a webhook is registered. Clear any webhook
    /// once at startup so an inherited bot (e.g. one previously driven by a
    /// webhook) can be polled. Best-effort.
    async fn delete_webhook(&self) {
        let url = format!("{}/bot{}/deleteWebhook", self.api_base, self.bot_token);
        if let Err(e) = self.http.post(&url).send().await {
            tracing::warn!(error = %e.without_url(), "telegram deleteWebhook failed (continuing)");
        }
    }

    async fn get_updates(&self, offset: Option<i64>) -> Result<Vec<Update>, GatewayError> {
        let url = format!("{}/bot{}/getUpdates", self.api_base, self.bot_token);
        let timeout = POLL_TIMEOUT_SECS.to_string();
        let mut query: Vec<(&str, String)> = vec![("timeout", timeout)];
        if let Some(o) = offset {
            query.push(("offset", o.to_string()));
        }
        let resp = self
            .http
            .get(&url)
            .query(&query)
            .send()
            .await
            .map_err(|e| GatewayError::Channel(format!("getUpdates: {}", e.without_url())))?;
        if !resp.status().is_success() {
            return Err(GatewayError::Channel(format!(
                "getUpdates HTTP {}",
                resp.status()
            )));
        }
        let parsed: GetUpdates = resp
            .json()
            .await
            .map_err(|e| GatewayError::Channel(format!("getUpdates decode: {e}")))?;
        Ok(parsed.result)
    }
}

#[async_trait]
impl Gateway for TelegramPollAdapter {
    fn channel(&self) -> ChannelId {
        "telegram".into()
    }

    fn ingress(&self) -> Ingress {
        Ingress::Poller
    }

    fn reply_sink(&self, correlation: &Correlation) -> Box<dyn ReplySink> {
        telegram_sink(
            self.http.clone(),
            self.api_base.clone(),
            self.bot_token.clone(),
            correlation.reply_to.clone(),
        )
    }

    async fn deliver_alert(&self, to: &Recipient, body: &AlertBody) -> Result<(), GatewayError> {
        telegram_send_alert(&self.http, &self.api_base, &self.bot_token, &to.to, &body.render()).await
    }
}

#[async_trait]
impl PollingGateway for TelegramPollAdapter {
    async fn run_ingress(&self, handler: Arc<dyn InboundHandler>) -> Result<(), GatewayError> {
        self.delete_webhook().await;
        // Resume from the persisted offset (Telegram acks updates <= offset-1).
        let mut offset: Option<i64> = self.cursor.load().await.and_then(|s| s.parse().ok());
        tracing::info!(?offset, "telegram poller started (getUpdates)");

        loop {
            let updates = match self.get_updates(offset).await {
                Ok(u) => u,
                Err(e) => {
                    tracing::warn!(error = %e, "telegram getUpdates failed — backing off");
                    tokio::time::sleep(POLL_BACKOFF).await;
                    continue;
                }
            };
            for update in updates {
                let next = update.update_id + 1;
                match update_to_inbound(update) {
                    Ok(msg) => {
                        let sink = self.reply_sink(&msg.correlation);
                        handler.handle(msg, sink).await;
                    }
                    Err(GatewayError::Ignored(_)) => {}
                    Err(e) => tracing::warn!(error = %e, "telegram update parse error"),
                }
                // Advance + persist the offset AFTER handling so a crash mid-turn
                // re-delivers the message rather than dropping it.
                offset = Some(next);
                self.cursor.save(&next.to_string()).await;
            }
        }
    }
}

/// The batched outbound side — one `sendMessage` per turn (FR-GW-02/12).
struct TelegramOutbound {
    http: reqwest::Client,
    api_base: String,
    bot_token: String,
    chat_id: String,
}

#[async_trait]
impl OutboundChannel for TelegramOutbound {
    async fn send(&self, text: String) -> Result<(), String> {
        let url = format!("{}/bot{}/sendMessage", self.api_base, self.bot_token);
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "chat_id": self.chat_id, "text": text }))
            .send()
            .await
            // FR-GW-17: the bot token is in the URL path (Telegram's API design),
            // and reqwest appends the request URL to a transport error's Display.
            // `without_url()` strips it so a routine network blip does not write
            // the token to the fail-soft warn! log in BufferSink::finish.
            .map_err(|e| format!("telegram sendMessage: {}", e.without_url()))?;
        if !resp.status().is_success() {
            return Err(format!("telegram sendMessage HTTP {}", resp.status()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reply_sink::TurnEvent;
    use std::collections::HashMap;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SECRET: &str = "webhook-secret-xyz";
    const TOKEN: &str = "123456:BOT-TOKEN";

    fn update_body(text: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 10,
                "from": { "id": 777, "is_bot": false, "first_name": "Wak" },
                "chat": { "id": 555, "type": "private" },
                "date": 1600000000,
                "text": text
            }
        }))
        .unwrap()
    }

    fn req_with_secret(secret: Option<&str>, body: Vec<u8>) -> InboundHttp {
        let mut h = HashMap::new();
        if let Some(s) = secret {
            h.insert(SECRET_HEADER.to_string(), s.to_string());
        }
        InboundHttp::new(h, body)
    }

    #[tokio::test]
    async fn valid_secret_and_text_parses() {
        // AC-P4-03 (parse half): valid secret + text update → InboundMessage.
        let adapter = TelegramAdapter::new(TOKEN.into(), SECRET.into());
        let msg = adapter
            .verify_and_parse(&req_with_secret(Some(SECRET), update_body("show firewall rules")))
            .await
            .expect("should parse");
        assert_eq!(msg.channel, "telegram");
        assert_eq!(msg.platform_handle, "777"); // sender id, for binding
        assert_eq!(msg.text, "show firewall rules");
        assert_eq!(msg.correlation.reply_to, "555"); // chat id, for reply
        assert_eq!(msg.correlation.session_id("telegram"), "gw:telegram:555");
    }

    #[tokio::test]
    async fn deliver_alert_posts_to_recipient_chat() {
        // P6-9 (FR-GW-13): deliver_alert sends one sendMessage to the explicit
        // recipient chat with the rendered title+body.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{TOKEN}/sendMessage")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = TelegramAdapter::with_api_base(TOKEN.into(), SECRET.into(), server.uri());
        let to = Recipient { channel: "telegram".into(), to: "555".into() };
        let body = AlertBody { title: "⚠ anomaly".into(), body: "cpu high on target://k3s-lab".into() };
        adapter.deliver_alert(&to, &body).await.expect("alert delivered");
        // The `.expect(1)` on the mock verifies exactly one sendMessage on drop.
    }

    #[tokio::test]
    async fn bad_secret_rejected_before_parse() {
        // AC-P4-04: wrong secret → Unauthorized, nothing parsed/dispatched.
        let adapter = TelegramAdapter::new(TOKEN.into(), SECRET.into());
        let err = adapter
            .verify_and_parse(&req_with_secret(Some("wrong"), update_body("hi")))
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn missing_secret_rejected() {
        let adapter = TelegramAdapter::new(TOKEN.into(), SECRET.into());
        let err = adapter
            .verify_and_parse(&req_with_secret(None, update_body("hi")))
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn non_text_update_ignored() {
        let adapter = TelegramAdapter::new(TOKEN.into(), SECRET.into());
        // An update with no `message` at all.
        let body = serde_json::to_vec(&serde_json::json!({ "update_id": 2 })).unwrap();
        let err = adapter
            .verify_and_parse(&req_with_secret(Some(SECRET), body))
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::Ignored(_)));
    }

    #[tokio::test]
    async fn send_error_does_not_leak_bot_token() {
        // FR-GW-17 regression: a transport error must not carry the bot token
        // (which is in the URL path) into the error string that gets logged.
        // Port 1 on loopback refuses immediately → a connect error whose URL,
        // if not stripped, would contain the token.
        let out = TelegramOutbound {
            http: reqwest::Client::new(),
            api_base: "http://127.0.0.1:1".into(),
            bot_token: TOKEN.into(),
            chat_id: "555".into(),
        };
        let err = out.send("hi".into()).await.unwrap_err();
        assert!(!err.contains(TOKEN), "bot token leaked in transport error: {err}");
        assert!(!err.contains("BOT-TOKEN"), "bot token fragment leaked: {err}");
    }

    #[tokio::test]
    async fn reply_sink_posts_sendmessage() {
        // AC-P4-03 (reply half): the batched sink flushes one sendMessage with
        // the coalesced text to the right chat id.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{TOKEN}/sendMessage")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = TelegramAdapter::with_api_base(TOKEN.into(), SECRET.into(), server.uri());
        let corr = Correlation {
            thread: Some("555".into()),
            reply_to: "555".into(),
        };
        let mut sink = adapter.reply_sink(&corr);
        sink.emit(TurnEvent::TokenDelta {
            session_id: "gw:telegram:555".into(),
            content: "6 drop rules on the edge firewall.".into(),
        })
        .await;
        sink.emit(TurnEvent::Done {
            session_id: "gw:telegram:555".into(),
            stop_reason: "end_turn".into(),
            input_tokens: 5,
            output_tokens: 8,
        })
        .await;
        sink.finish().await;

        // Wiremock's `.expect(1)` verifies exactly one sendMessage on drop.
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1);
        let body: serde_json::Value = reqs[0].body_json().unwrap();
        assert_eq!(body["chat_id"], "555");
        assert_eq!(body["text"], "6 drop rules on the edge firewall.");
    }

    #[derive(Default)]
    struct TestCursor {
        v: std::sync::Mutex<Option<String>>,
    }
    #[async_trait]
    impl CursorStore for TestCursor {
        async fn load(&self) -> Option<String> {
            self.v.lock().unwrap().clone()
        }
        async fn save(&self, c: &str) {
            *self.v.lock().unwrap() = Some(c.to_string());
        }
    }

    #[tokio::test]
    async fn poll_get_updates_parses_and_normalises() {
        // The poller ingress: getUpdates returns a text update → parses to an
        // InboundMessage with the same handle/reply mapping as the webhook path,
        // and the offset advances past the update_id.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/bot{TOKEN}/getUpdates")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": [
                    { "update_id": 42, "message": {
                        "message_id": 1, "from": { "id": 777 }, "chat": { "id": 555 }, "text": "list drop rules"
                    } }
                ]
            })))
            .mount(&server)
            .await;
        let store: std::sync::Arc<dyn CursorStore> = std::sync::Arc::new(TestCursor::default());
        let a = TelegramPollAdapter::with_api_base(TOKEN.into(), server.uri(), store);
        let updates = a.get_updates(None).await.unwrap();
        assert_eq!(updates.len(), 1);
        let next_offset = updates[0].update_id + 1;
        assert_eq!(next_offset, 43);
        let msg = update_to_inbound(updates.into_iter().next().unwrap()).unwrap();
        assert_eq!(msg.channel, "telegram");
        assert_eq!(msg.platform_handle, "777");
        assert_eq!(msg.text, "list drop rules");
        assert_eq!(msg.correlation.reply_to, "555");
    }
}
