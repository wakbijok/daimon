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

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

use crate::gateway::{
    ChannelId, Correlation, Gateway, GatewayError, InboundHttp, InboundMessage, Ingress,
};
use crate::reply_sink::{BufferSink, OutboundChannel, ReplySink};
use crate::verify;

const DEFAULT_API_BASE: &str = "https://api.telegram.org";
const SECRET_HEADER: &str = "x-telegram-bot-api-secret-token";

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

#[async_trait]
impl Gateway for TelegramAdapter {
    fn channel(&self) -> ChannelId {
        "telegram".into()
    }

    fn ingress(&self) -> Ingress {
        Ingress::Webhook
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

        // A webhook fires for many update kinds (edits, joins, callbacks). We
        // only act on a text message; everything else is acked-and-ignored.
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

        // Bind on the sender's numeric user id (stable across username changes);
        // reply to the chat the message arrived in (differs from the sender in a
        // group chat — bind the person, answer the room).
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

    fn reply_sink(&self, correlation: &Correlation) -> Box<dyn ReplySink> {
        Box::new(BufferSink::new(TelegramOutbound {
            http: self.http.clone(),
            api_base: self.api_base.clone(),
            bot_token: self.bot_token.clone(),
            chat_id: correlation.reply_to.clone(),
        }))
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
            .map_err(|e| format!("telegram sendMessage: {e}"))?;
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
}
