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
    Gateway, GatewayError, InboundHandler, InboundHttp, InboundMessage, PollingGateway, ReplySink,
    TurnEvent,
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

        // P7-2 (FR-GW-14): reply-to correlation. If this message REPLIES to an
        // approval alert and its body is a bare approve/deny, resolve the approval
        // id from the persisted delivery correlation and decide — the approver
        // never needs to type the id. Same server-side authority check applies.
        if let Some(reply_id) = msg.reply_to_message_id.as_deref() {
            if let Some(approved) = parse_bare_decision(&msg.text) {
                match crate::db::resolve_approval_by_reply(&self.state.db, &msg.channel, reply_id)
                    .await
                {
                    Ok(Some(approval_id)) => {
                        self.decide_over_chat(&actor, approved, approval_id, &session_id, &mut sink)
                            .await;
                        sink.finish().await;
                        return;
                    }
                    // Reply to a non-approval message → fall through to normal chat.
                    Ok(None) => {}
                    Err(e) => {
                        warn!(error = %e, "approve-over-chat: reply correlation lookup failed");
                    }
                }
            }
        }

        // P6-12 (FR-GW-14): approve-over-chat. If the message is an approve/deny
        // command WITH an explicit id, apply the decision through the SAME
        // server-side role check as the console (approver/admin) — the bound
        // channel identity NEVER grants authority — instead of dispatching it to
        // the LLM.
        if let Some((approved, approval_id)) = parse_approval_command(&msg.text) {
            self.decide_over_chat(&actor, approved, approval_id, &session_id, &mut sink)
                .await;
            sink.finish().await;
            return;
        }

        // The SAME entry point a browser turn uses (FR-GW-09/10). It calls
        // `sink.finish()` itself at end-of-turn, flushing the batched reply.
        crate::chat::handle_chat_send(
            &mut *sink,
            &self.state,
            &actor_id,
            session_id,
            msg.text,
            None,
            None,
        )
        .await;
    }
}

impl AppInboundHandler {
    /// Apply an approve/deny decision received over a gateway (FR-GW-14). The
    /// authority check is IDENTICAL to the console `decide_approval` server-fn
    /// (`require_approver`: the actor must hold `approver` or `admin`) and runs
    /// server-side against the fail-closed-bound identity — a read-only/operator
    /// reply is refused. Does NOT trust any client-supplied actor; uses the
    /// bound `GatewayActor`.
    async fn decide_over_chat(
        &self,
        actor: &crate::db::GatewayActor,
        approved: bool,
        approval_id: uuid::Uuid,
        session_id: &str,
        sink: &mut Box<dyn ReplySink>,
    ) {
        let is_approver = actor
            .roles
            .iter()
            .any(|r| r == "approver" || r == "admin");
        if !is_approver {
            warn!(actor = %actor.username, "approve-over-chat refused — not an approver/admin");
            reply(sink, session_id,
                "🚫 Not authorized: approving requires the `approver` or `admin` role.").await;
            return;
        }

        // The guard's ApprovalQueue is the single decision writer — the broker's
        // parked `execute` is watching the same row (identical to a console
        // decision). Reached via the broker, never a direct vault/db path.
        let Some(queue) = self.state.broker.guard().map(|g| g.approvals().clone()) else {
            reply(sink, session_id, "internal error: no guard configured").await;
            return;
        };
        let status = if approved {
            daimon_guard::ApprovalStatus::Approved
        } else {
            daimon_guard::ApprovalStatus::Denied
        };
        match queue.decide(approval_id, actor.user_id, status).await {
            Ok(_) => {
                info!(
                    actor = %actor.username,
                    approval = %approval_id,
                    approved,
                    "approve-over-chat decision applied"
                );
                let verb = if approved { "✅ Approved" } else { "🛑 Denied" };
                reply(sink, session_id, &format!("{verb} approval `{approval_id}`.")).await;
            }
            // decide() only touches a `pending` row, so a missing row means the
            // approval was already decided, expired, or the id is wrong.
            Err(e) => {
                warn!(approval = %approval_id, error = %e, "approve-over-chat decide failed");
                reply(sink, session_id, &format!(
                    "No pending approval `{approval_id}` (already decided, expired, or unknown id)."
                )).await;
            }
        }
    }
}

/// Parse an `approve <uuid>` / `deny <uuid>` command (case-insensitive, optional
/// leading `/`). Returns `(approved, id)` or `None` if the text is a normal
/// chat message. This is the explicit-id form; reply-to correlation layers on
/// top of it by resolving the replied-to alert message to its approval id.
fn parse_approval_command(text: &str) -> Option<(bool, uuid::Uuid)> {
    let t = text.trim();
    let (verb, rest) = t.split_once(char::is_whitespace)?;
    let approved = match verb.to_ascii_lowercase().as_str() {
        "approve" | "/approve" => true,
        "deny" | "/deny" => false,
        _ => return None,
    };
    let id = uuid::Uuid::parse_str(rest.trim()).ok()?;
    Some((approved, id))
}

/// Parse a BARE approve/deny reply (no id — the id comes from reply correlation,
/// P7-2). Strips a Matrix reply-quote fallback (`> …` lines) before matching, so
/// `> <@daimon> approval required\n\napprove` reads as `approve`. Returns
/// `Some(true/false)` or `None` if the reply is not a decision.
fn parse_bare_decision(text: &str) -> Option<bool> {
    let cleaned: String = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('>'))
        .collect::<Vec<_>>()
        .join(" ");
    let first = cleaned.split_whitespace().next()?;
    match first.to_ascii_lowercase().as_str() {
        "approve" | "/approve" | "approved" => Some(true),
        "deny" | "/deny" | "denied" => Some(false),
        _ => None,
    }
}

/// Emit a single-message reply through a batched sink (one `TokenDelta`).
async fn reply(sink: &mut Box<dyn ReplySink>, session_id: &str, text: &str) {
    sink.emit(TurnEvent::TokenDelta {
        session_id: session_id.to_string(),
        content: text.to_string(),
    })
    .await;
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

// ---- Poller wiring (P4-5 Matrix /sync, P4-8 Telegram getUpdates) ------------

/// A poller's resume cursor, persisted in `app_config` under `key` so a restart
/// does not reprocess history. Backs `daimon_gateway::CursorStore` for both
/// Matrix (`channels.matrix.since`) and Telegram (`channels.telegram.offset`).
/// daimon-gateway has no DB access (D21), so the store is injected here.
pub struct AppConfigCursor {
    pool: daimon_db::Pool,
    key: String,
}

impl AppConfigCursor {
    pub fn new(pool: daimon_db::Pool, key: impl Into<String>) -> Self {
        Self {
            pool,
            key: key.into(),
        }
    }
}

#[async_trait]
impl daimon_gateway::CursorStore for AppConfigCursor {
    async fn load(&self) -> Option<String> {
        crate::db::get_config(&self.pool, &self.key)
            .await
            .ok()
            .flatten()
    }

    async fn save(&self, cursor: &str) {
        if let Err(e) = crate::db::set_config(&self.pool, &self.key, cursor).await {
            warn!(error = %e, key = %self.key, "failed to persist poller cursor");
        }
    }
}

/// Spawn a supervised poller (Matrix `/sync` or Telegram `getUpdates`).
/// `run_ingress` only returns on a fatal error; the wrapper restarts it after a
/// backoff so a transient outage self-heals. Called at boot only when the
/// channel is enabled + its token resolved.
pub fn spawn_poller(state: AppState, adapter: Arc<dyn daimon_gateway::PollingGateway>) {
    let ch = adapter.channel();
    let handler: Arc<dyn InboundHandler> = Arc::new(AppInboundHandler::new(state));
    tokio::spawn(async move {
        loop {
            match adapter.run_ingress(handler.clone()).await {
                Ok(()) => warn!(channel = %ch, "poller returned cleanly — restarting in 10s"),
                Err(e) => error!(channel = %ch, error = %e, "poller exited — restarting in 10s"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{parse_approval_command, parse_bare_decision};

    #[test]
    fn bare_decision_strips_matrix_quote_and_matches() {
        // plain reply
        assert_eq!(parse_bare_decision("approve"), Some(true));
        assert_eq!(parse_bare_decision("/deny"), Some(false));
        assert_eq!(parse_bare_decision("Approved please"), Some(true));
        // Matrix reply-quote fallback is stripped before matching
        assert_eq!(
            parse_bare_decision("> <@daimon:hs> 🔐 approval required\n\napprove"),
            Some(true)
        );
        // not a decision
        assert_eq!(parse_bare_decision("what does this do?"), None);
        assert_eq!(parse_bare_decision("> only a quote"), None);
    }

    #[test]
    fn parses_approve_and_deny_with_uuid() {
        let id = "11111111-2222-3333-4444-555555555555";
        let (ok, got) = parse_approval_command(&format!("approve {id}")).unwrap();
        assert!(ok);
        assert_eq!(got.to_string(), id);
        let (ok, _) = parse_approval_command(&format!("/DENY {id}")).unwrap();
        assert!(!ok);
    }

    #[test]
    fn ignores_non_commands_and_bad_ids() {
        assert!(parse_approval_command("what firewall rules do you have?").is_none());
        assert!(parse_approval_command("approve not-a-uuid").is_none());
        assert!(parse_approval_command("approve").is_none()); // no id
    }
}
