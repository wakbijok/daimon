//! Matrix adapter — `/sync` long-poll poller (SDS §9 addendum: poller ingress).
//!
//! Unlike Telegram's webhook, a Matrix bot receives messages by long-polling the
//! homeserver's client-server `/sync` endpoint with its access token. There is
//! no per-request signature — authenticity is the **bot access token on the
//! connection** (resolved from the vault by reference at boot, FR-GW-17). This is
//! the homelab-simplest ingress: a bot account + an access token, no homeserver-
//! side Application Service registration.
//!
//! The loop:
//! 1. `whoami` once to learn the bot's own MXID (skip self-echo — else the bot
//!    answers its own replies forever).
//! 2. Resume from the persisted `since` cursor (via the injected
//!    [`SyncCursorStore`], backed by `app_config` in daimon-app) so a restart
//!    does not reprocess history; on a cold start, an initial `since`-less sync
//!    seeds the cursor and its backlog is skipped.
//! 3. Each `m.room.message` / `m.text` event from another sender is normalised to
//!    an [`InboundMessage`] and handed to the shared [`InboundHandler`] — the
//!    SAME pipeline a webhook message takes (FR-GW-09). Replies go back via a
//!    batched [`BufferSink`] over the room-send endpoint (FR-GW-12).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

use crate::gateway::{
    ChannelId, Correlation, Gateway, GatewayError, InboundHandler, InboundHttp, InboundMessage,
    Ingress, PollingGateway,
};
use crate::reply_sink::{BufferSink, OutboundChannel, ReplySink};

const SYNC_TIMEOUT_MS: u64 = 30_000;
const BACKOFF: Duration = Duration::from_secs(5);

/// Persists the `/sync` resume cursor across restarts. daimon-gateway has no DB
/// access (D21), so the store is injected — daimon-app backs it with
/// `app_config` key `channels.matrix.since`.
#[async_trait]
pub trait SyncCursorStore: Send + Sync {
    async fn load(&self) -> Option<String>;
    async fn save(&self, cursor: &str);
}

/// A Matrix bot as a daimon `Gateway` (poller ingress).
pub struct MatrixAdapter {
    homeserver: String,
    access_token: String,
    http: reqwest::Client,
    cursor: Arc<dyn SyncCursorStore>,
    txn: Arc<AtomicU64>,
}

impl MatrixAdapter {
    pub fn new(
        homeserver: String,
        access_token: String,
        cursor: Arc<dyn SyncCursorStore>,
    ) -> Self {
        Self {
            homeserver: homeserver.trim_end_matches('/').to_string(),
            access_token,
            http: reqwest::Client::new(),
            cursor,
            txn: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Resolve the bot's own MXID (`GET /account/whoami`) — used to skip
    /// self-authored events.
    async fn whoami(&self) -> Result<String, GatewayError> {
        #[derive(Deserialize)]
        struct WhoAmI {
            user_id: String,
        }
        let url = format!("{}/_matrix/client/v3/account/whoami", self.homeserver);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await
            .map_err(|e| GatewayError::Channel(format!("whoami: {e}")))?;
        if !resp.status().is_success() {
            return Err(GatewayError::Unauthorized(format!(
                "matrix whoami HTTP {}",
                resp.status()
            )));
        }
        let who: WhoAmI = resp
            .json()
            .await
            .map_err(|e| GatewayError::Channel(format!("whoami decode: {e}")))?;
        Ok(who.user_id)
    }

    /// One `/sync` call. `since = None` is the cold-start sync (its backlog is
    /// skipped by the caller; only `next_batch` is kept).
    async fn fetch_sync(&self, since: Option<&str>) -> Result<SyncResponse, GatewayError> {
        let url = format!("{}/_matrix/client/v3/sync", self.homeserver);
        let mut req = self.http.get(&url).bearer_auth(&self.access_token);
        let timeout = if since.is_some() { SYNC_TIMEOUT_MS } else { 0 };
        let timeout_s = timeout.to_string();
        let mut query: Vec<(&str, &str)> = vec![("timeout", &timeout_s)];
        if let Some(s) = since {
            query.push(("since", s));
        }
        req = req.query(&query);
        let resp = req
            .send()
            .await
            .map_err(|e| GatewayError::Channel(format!("sync: {e}")))?;
        if !resp.status().is_success() {
            return Err(GatewayError::Channel(format!(
                "matrix sync HTTP {}",
                resp.status()
            )));
        }
        resp.json::<SyncResponse>()
            .await
            .map_err(|e| GatewayError::Channel(format!("sync decode: {e}")))
    }

    /// Normalise a sync's joined-room text messages and dispatch each (skipping
    /// this bot's own). Extracted from the loop so it is unit-testable.
    async fn dispatch_sync(
        &self,
        sync: &SyncResponse,
        bot_id: &str,
        handler: &Arc<dyn InboundHandler>,
    ) {
        for (room_id, text, sender) in sync.text_messages() {
            if sender == bot_id {
                continue; // skip self-echo
            }
            let msg = InboundMessage {
                channel: "matrix".into(),
                platform_handle: sender,
                text,
                correlation: Correlation {
                    thread: Some(room_id.clone()),
                    reply_to: room_id,
                },
                received_at: Utc::now(),
            };
            let sink = self.reply_sink(&msg.correlation);
            handler.handle(msg, sink).await;
        }
    }
}

#[async_trait]
impl Gateway for MatrixAdapter {
    fn channel(&self) -> ChannelId {
        "matrix".into()
    }

    fn ingress(&self) -> Ingress {
        Ingress::Poller
    }

    // Poller adapter: `verify_and_parse` (webhook contract) is intentionally
    // unimplemented — inherits the default `NotImplemented`. Authenticity is the
    // access token on the /sync connection, not a per-request signature.
    async fn verify_and_parse(&self, _req: &InboundHttp) -> Result<InboundMessage, GatewayError> {
        Err(GatewayError::NotImplemented)
    }

    fn reply_sink(&self, correlation: &Correlation) -> Box<dyn ReplySink> {
        Box::new(BufferSink::new(MatrixOutbound {
            http: self.http.clone(),
            homeserver: self.homeserver.clone(),
            access_token: self.access_token.clone(),
            room_id: correlation.reply_to.clone(),
            txn: self.txn.clone(),
        }))
    }
}

#[async_trait]
impl PollingGateway for MatrixAdapter {
    async fn run_ingress(&self, handler: Arc<dyn InboundHandler>) -> Result<(), GatewayError> {
        let bot_id = self.whoami().await?;
        tracing::info!(bot = %bot_id, homeserver = %self.homeserver, "matrix poller started");

        // Resume, or cold-start (seed the cursor, skip backlog).
        let mut since = match self.cursor.load().await {
            Some(s) => s,
            None => {
                let initial = self.fetch_sync(None).await?;
                self.cursor.save(&initial.next_batch).await;
                initial.next_batch
            }
        };

        loop {
            let sync = match self.fetch_sync(Some(&since)).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "matrix sync failed — backing off");
                    tokio::time::sleep(BACKOFF).await;
                    continue;
                }
            };
            self.dispatch_sync(&sync, &bot_id, &handler).await;
            since = sync.next_batch.clone();
            self.cursor.save(&since).await;
        }
    }
}

/// The batched outbound side — one room message per turn (FR-GW-02/12).
struct MatrixOutbound {
    http: reqwest::Client,
    homeserver: String,
    access_token: String,
    room_id: String,
    txn: Arc<AtomicU64>,
}

#[async_trait]
impl OutboundChannel for MatrixOutbound {
    async fn send(&self, text: String) -> Result<(), String> {
        let txn = self.txn.fetch_add(1, Ordering::Relaxed);
        let url = format!(
            "{}/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
            self.homeserver,
            urlencode(&self.room_id),
            txn
        );
        let resp = self
            .http
            .put(&url)
            .bearer_auth(&self.access_token)
            .json(&serde_json::json!({ "msgtype": "m.text", "body": text }))
            .send()
            .await
            .map_err(|e| format!("matrix room send: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("matrix room send HTTP {}", resp.status()));
        }
        Ok(())
    }
}

/// Minimal percent-encoding for a Matrix room id in a path segment (`!`, `:`,
/// `/`). Room ids look like `!abc:server` — the `!` and `:` must be encoded.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---- /sync wire types (partial — only what we consume) ----

#[derive(Deserialize)]
struct SyncResponse {
    next_batch: String,
    #[serde(default)]
    rooms: Rooms,
}

#[derive(Deserialize, Default)]
struct Rooms {
    #[serde(default)]
    join: HashMap<String, JoinedRoom>,
}

#[derive(Deserialize, Default)]
struct JoinedRoom {
    #[serde(default)]
    timeline: Timeline,
}

#[derive(Deserialize, Default)]
struct Timeline {
    #[serde(default)]
    events: Vec<Event>,
}

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    sender: String,
    #[serde(default)]
    content: EventContent,
}

#[derive(Deserialize, Default)]
struct EventContent {
    #[serde(default)]
    msgtype: Option<String>,
    #[serde(default)]
    body: Option<String>,
}

impl SyncResponse {
    /// `(room_id, text, sender)` for every `m.room.message` / `m.text` event.
    fn text_messages(&self) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for (room_id, room) in &self.rooms.join {
            for ev in &room.timeline.events {
                if ev.kind != "m.room.message" {
                    continue;
                }
                if ev.content.msgtype.as_deref() != Some("m.text") {
                    continue;
                }
                if let Some(body) = ev.content.body.as_deref() {
                    if !body.trim().is_empty() {
                        out.push((room_id.clone(), body.to_string(), ev.sender.clone()));
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reply_sink::TurnEvent;
    use std::sync::Mutex;
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TOKEN: &str = "matrix-access-token";
    const BOT: &str = "@daimon:hs.example";

    struct NullCursor;
    #[async_trait]
    impl SyncCursorStore for NullCursor {
        async fn load(&self) -> Option<String> {
            None
        }
        async fn save(&self, _cursor: &str) {}
    }

    /// Records every dispatched inbound message; flushes each sink so any
    /// outbound is exercised too.
    #[derive(Default)]
    struct RecordingHandler {
        seen: Arc<Mutex<Vec<InboundMessage>>>,
    }
    #[async_trait]
    impl InboundHandler for RecordingHandler {
        async fn handle(&self, msg: InboundMessage, mut sink: Box<dyn ReplySink>) {
            self.seen.lock().unwrap().push(msg);
            sink.finish().await;
        }
    }

    fn adapter(server: &MockServer) -> MatrixAdapter {
        MatrixAdapter::new(server.uri(), TOKEN.into(), Arc::new(NullCursor))
    }

    #[test]
    fn text_messages_filters_and_extracts() {
        let sync: SyncResponse = serde_json::from_value(serde_json::json!({
            "next_batch": "s2",
            "rooms": { "join": { "!room:hs": { "timeline": { "events": [
                { "type": "m.room.message", "sender": "@wak:hs", "content": { "msgtype": "m.text", "body": "list drop rules" } },
                { "type": "m.room.message", "sender": "@wak:hs", "content": { "msgtype": "m.image", "body": "pic" } },
                { "type": "m.room.member", "sender": "@wak:hs", "content": {} }
            ] } } } }
        })).unwrap();
        let msgs = sync.text_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].0, "!room:hs");
        assert_eq!(msgs[0].1, "list drop rules");
        assert_eq!(msgs[0].2, "@wak:hs");
    }

    #[tokio::test]
    async fn whoami_and_sync_over_wiremock() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/_matrix/client/v3/account/whoami"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"user_id": BOT})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/_matrix/client/v3/sync"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "next_batch": "s99",
                "rooms": { "join": { "!r:hs": { "timeline": { "events": [
                    { "type": "m.room.message", "sender": "@wak:hs", "content": { "msgtype": "m.text", "body": "hi daimon" } }
                ] } } } }
            })))
            .mount(&server)
            .await;

        let a = adapter(&server);
        assert_eq!(a.whoami().await.unwrap(), BOT);
        let sync = a.fetch_sync(Some("s1")).await.unwrap();
        assert_eq!(sync.next_batch, "s99");
        assert_eq!(sync.text_messages().len(), 1);
    }

    #[tokio::test]
    async fn dispatch_skips_self_echo() {
        // AC-P4-07 (self-echo half): a message from the bot itself is skipped;
        // one from another sender is dispatched.
        let server = MockServer::start().await;
        let a = adapter(&server);
        let sync: SyncResponse = serde_json::from_value(serde_json::json!({
            "next_batch": "s3",
            "rooms": { "join": { "!r:hs": { "timeline": { "events": [
                { "type": "m.room.message", "sender": BOT, "content": { "msgtype": "m.text", "body": "my own reply" } },
                { "type": "m.room.message", "sender": "@wak:hs", "content": { "msgtype": "m.text", "body": "operator question" } }
            ] } } } }
        })).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<dyn InboundHandler> = Arc::new(RecordingHandler { seen: seen.clone() });
        a.dispatch_sync(&sync, BOT, &handler).await;
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "self message skipped, operator message kept");
        assert_eq!(seen[0].platform_handle, "@wak:hs");
        assert_eq!(seen[0].text, "operator question");
        assert_eq!(seen[0].correlation.reply_to, "!r:hs");
    }

    #[tokio::test]
    async fn reply_sink_puts_room_message() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex(r"^/_matrix/client/v3/rooms/.+/send/m\.room\.message/.+$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"event_id": "$e"})))
            .expect(1)
            .mount(&server)
            .await;
        let a = adapter(&server);
        let corr = Correlation {
            thread: Some("!r:hs".into()),
            reply_to: "!r:hs".into(),
        };
        let mut sink = a.reply_sink(&corr);
        sink.emit(TurnEvent::TokenDelta {
            session_id: "gw:matrix:!r:hs".into(),
            content: "edge firewall has 6 drop rules.".into(),
        })
        .await;
        sink.finish().await;
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1);
        let body: serde_json::Value = reqs[0].body_json().unwrap();
        assert_eq!(body["msgtype"], "m.text");
        assert_eq!(body["body"], "edge firewall has 6 drop rules.");
    }
}
