//! Messaging-gateway wiring (P4, SRS §4.8 / SDS §9.4).
//!
//! This is the app-side glue between `daimon-gateway`'s channel adapters and the
//! chat turn loop:
//!
//! - [`GatewayRegistry`] — the enabled webhook adapters, keyed by channel id.
//!   Poller adapters (Matrix, P4-5) are supervised separately at boot; this holds
//!   the webhook-ingress adapters the inbound HTTP route dispatches to.
//! - [`AppInboundHandler`] — the single inbound pipeline both a webhook route and
//!   a poller funnel through: **bind identity (fail-closed) → run the SAME chat
//!   turn a browser message runs → reply flows back out the `ReplySink`**. There
//!   is no parallel executor; a gateway is a front door to the existing Harness
//!   (FR-GW-09/10/11).
//! - [`gateway_webhook`] — the `POST /api/v1/gw/{channel}` axum handler: verify
//!   (FR-GW-07) → 401 on failure, else ack fast + process the turn in the
//!   background (so a slow LLM turn never trips Telegram's webhook retry).

#![cfg(feature = "ssr")]

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::{Extension, Path};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use daimon_gateway::{
    Gateway, GatewayError, InboundHandler, InboundHttp, InboundMessage, ReplySink, TurnEvent,
};
use tracing::{error, info, warn};

use crate::state::AppState;

/// The enabled webhook adapters, keyed by channel id (`"telegram"`, …).
#[derive(Default)]
pub struct GatewayRegistry {
    webhooks: HashMap<String, Arc<dyn Gateway>>,
}

impl GatewayRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a webhook adapter under its `channel()` id.
    pub fn register(&mut self, adapter: Arc<dyn Gateway>) {
        self.webhooks.insert(adapter.channel(), adapter);
    }

    /// Look up an adapter for an inbound route.
    pub fn get(&self, channel: &str) -> Option<Arc<dyn Gateway>> {
        self.webhooks.get(channel).cloned()
    }

    /// The enabled webhook channel ids (for the Channels settings surface).
    pub fn enabled_channels(&self) -> Vec<String> {
        let mut v: Vec<String> = self.webhooks.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn is_empty(&self) -> bool {
        self.webhooks.is_empty()
    }
}

/// The shared inbound pipeline (SDS §9.4.3). Holds an `AppState` and runs the
/// bind-identity → dispatch → reply loop. Both the webhook route and the Matrix
/// poller construct one and call [`InboundHandler::handle`].
pub struct AppInboundHandler {
    state: AppState,
}

impl AppInboundHandler {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl InboundHandler for AppInboundHandler {
    async fn handle(&self, msg: InboundMessage, mut sink: Box<dyn ReplySink>) {
        // FR-GW-08: fail-closed identity binding. An unmapped platform handle —
        // or one bound to a non-active account — is refused; NO capability runs.
        // This is the gateway equivalent of the C4 hardcoded-"operator" fix.
        let actor = match crate::db::resolve_gateway_identity(
            &self.state.db,
            &msg.channel,
            &msg.platform_handle,
        )
        .await
        {
            Ok(Some(a)) => a,
            Ok(None) => {
                warn!(
                    channel = %msg.channel,
                    handle = %msg.platform_handle,
                    "gateway: unmapped platform handle — refused (no dispatch)"
                );
                sink.emit(TurnEvent::Error {
                    message: format!(
                        "Not authorized: your {} identity is not enrolled with daimon. \
                         Ask an admin to bind your handle.",
                        msg.channel
                    ),
                })
                .await;
                sink.finish().await;
                return;
            }
            Err(e) => {
                error!(error = %e, channel = %msg.channel, "gateway: identity resolution failed");
                sink.emit(TurnEvent::Error {
                    message: "internal error resolving identity".into(),
                })
                .await;
                sink.finish().await;
                return;
            }
        };

        // The resolved actor id matches the console's `format!("user:{}",
        // claims.sub)` exactly (claims.sub is the username), so a gateway turn is
        // attributed identically on the audit chain (FR-GW-11).
        let actor_id = format!("user:{}", actor.username);
        let session_id = msg.correlation.session_id(&msg.channel);
        info!(
            channel = %msg.channel,
            actor = %actor_id,
            session = %session_id,
            "gateway turn dispatched (same Harness/Guard path as a browser turn)"
        );

        // The SAME entry point a browser turn uses (FR-GW-09/10). It calls
        // `sink.finish()` itself at end-of-turn, flushing the batched reply.
        crate::chat::handle_chat_send(
            &mut *sink,
            &self.state,
            &actor_id,
            session_id,
            msg.text,
            None,
        )
        .await;
    }
}

/// `POST /api/v1/gw/{channel}` — the inbound webhook route for every webhook
/// adapter. Verifies authenticity first (FR-GW-07); on success it acks Telegram
/// immediately and runs the turn in a background task.
pub async fn gateway_webhook(
    Extension(state): Extension<AppState>,
    Path(channel): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let Some(adapter) = state.gateways.get(&channel) else {
        return (StatusCode::NOT_FOUND, "no such gateway").into_response();
    };

    let req = InboundHttp::new(header_map_to_hashmap(&headers), body.to_vec());
    match adapter.verify_and_parse(&req).await {
        Ok(msg) => {
            // Ack fast, process async: an inbound webhook must return 200 quickly
            // or Telegram retries and the operator gets duplicate turns.
            let sink = adapter.reply_sink(&msg.correlation);
            let handler = AppInboundHandler::new(state.clone());
            tokio::spawn(async move {
                handler.handle(msg, sink).await;
            });
            (StatusCode::OK, "ok").into_response()
        }
        Err(GatewayError::Unauthorized(m)) => {
            warn!(channel = %channel, "gateway inbound rejected (verification failed): {m}");
            (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
        }
        // A non-actionable update (edit, join, non-text) — ack so the platform
        // stops retrying, but run nothing.
        Err(GatewayError::Ignored(_)) => (StatusCode::OK, "ignored").into_response(),
        Err(GatewayError::BadRequest(m)) => (StatusCode::BAD_REQUEST, m).into_response(),
        Err(e) => {
            error!(channel = %channel, "gateway inbound error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "gateway error").into_response()
        }
    }
}

fn header_map_to_hashmap(h: &HeaderMap) -> HashMap<String, String> {
    h.iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|vs| (k.as_str().to_string(), vs.to_string())))
        .collect()
}

// ---- Matrix poller wiring (P4-5) --------------------------------------------

/// The Matrix `/sync` resume cursor, persisted in `app_config` so a restart does
/// not reprocess room history. Backs `daimon_gateway`'s `SyncCursorStore` (that
/// crate has no DB access — D21).
pub struct AppConfigCursor {
    pool: daimon_db::Pool,
}

impl AppConfigCursor {
    const KEY: &'static str = "channels.matrix.since";

    pub fn new(pool: daimon_db::Pool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl daimon_gateway::adapters::matrix::SyncCursorStore for AppConfigCursor {
    async fn load(&self) -> Option<String> {
        crate::db::get_config(&self.pool, Self::KEY)
            .await
            .ok()
            .flatten()
    }

    async fn save(&self, cursor: &str) {
        if let Err(e) = crate::db::set_config(&self.pool, Self::KEY, cursor).await {
            warn!(error = %e, "matrix: failed to persist /sync cursor");
        }
    }
}

/// Spawn the supervised Matrix poller. `run_ingress` only returns on a fatal
/// error (bad token, decode failure); the wrapper restarts it after a backoff so
/// a transient homeserver outage self-heals. Called at boot (P4-7) only when the
/// `matrix` channel is enabled and its access token resolved.
pub fn spawn_matrix_poller(
    state: AppState,
    adapter: Arc<daimon_gateway::adapters::matrix::MatrixAdapter>,
) {
    use daimon_gateway::PollingGateway;
    let handler: Arc<dyn InboundHandler> = Arc::new(AppInboundHandler::new(state));
    tokio::spawn(async move {
        loop {
            match adapter.run_ingress(handler.clone()).await {
                Ok(()) => {
                    warn!("matrix poller returned cleanly — restarting in 10s");
                }
                Err(e) => {
                    error!(error = %e, "matrix poller exited — restarting in 10s");
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
    });
}
