//! P6-10/11 — outbound alert routing (FR-GW-13/15/16).
//!
//! A single background task (`spawn_alert_router`) subscribes to the agent bus
//! and, for each routable event, delivers a proactive alert to the configured
//! channel(s) through the EXISTING gateway outbound seam (`Gateway::deliver_alert`,
//! P6-9) — there is no second sender. Two event classes route (FR-GW-13):
//!
//! - **anomaly** — the observer's `AnomalyDetected` envelope (already on the bus,
//!   broadcast; the router observes a copy without disturbing the TriageAgent).
//! - **approval** — a Guard `AwaitingApproval` envelope (emitted in P6-12).
//!
//! Three invariants:
//! - **fail-closed recipients** (FR-GW-16): a routing target is delivered ONLY if
//!   it resolves to an ENROLLED, ACTIVE gateway identity — an alert is never sent
//!   to an unbound handle.
//! - **fail-soft delivery** (FR-GW-15): a channel that is unreachable is logged +
//!   recorded in `alert_deliveries` and NEVER blocks the originating loop; the
//!   router only NOTIFIES, it never re-runs a capability.
//! - **dedup** (AC-P6): at most one alert per signature within a TTL window, so a
//!   flapping metric collapses to a single notification.

#![cfg(feature = "ssr")]

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use daimon_core::AgentEnvelope;
use daimon_gateway::gateway::{AlertBody, Recipient};
use tokio::sync::broadcast::error::RecvError;

use crate::state::AppState;

/// How long a delivered signature suppresses re-alerts (fixed for P6).
const DEDUP_TTL: Duration = Duration::from_secs(300);

/// A routable alert normalised from a bus envelope.
struct Alert {
    /// `"anomaly"` | `"approval"` — the config routing class.
    class: &'static str,
    /// `"critical"` / `"warning"` / … — used for per-severity routing.
    severity: String,
    /// The dedup key (anomaly id / plan id).
    signature: String,
    body: AlertBody,
}

/// Classify a bus envelope into a routable [`Alert`], or `None` if it is not an
/// event this router cares about. Deserialises the envelope body — the router
/// never depends on the concrete producer type.
fn classify(env: &AgentEnvelope) -> Option<Alert> {
    // anomaly — the observer's AnomalyDetected (addressed ByCapability the triage
    // name). Match the payload structurally, so a schema the router cannot read
    // is simply skipped, never a panic.
    if let Ok(a) = serde_json::from_value::<daimon_observer::AnomalyDetected>(env.body.clone()) {
        let body = AlertBody {
            title: format!("⚠ anomaly · {}", a.severity),
            body: format!(
                "{}\nmetric {} = {} (threshold {})\ntarget {}",
                a.title,
                a.metric_name,
                a.metric_value,
                a.threshold,
                a.target_ref.as_deref().unwrap_or(&a.source_id),
            ),
        };
        return Some(Alert {
            class: "anomaly",
            severity: a.severity,
            signature: a.anomaly_id.to_string(),
            body,
        });
    }
    // approval — a Guard AwaitingApproval envelope (P6-12 emits this shape).
    if let Some(approval) = classify_approval(env) {
        return Some(approval);
    }
    None
}

/// P6-12 shape: `{ "kind": "awaiting_approval", "approval_id", "capability",
/// "target_ref", "actor" }`. Kept lenient so the emit side (guard) and this
/// side stay loosely coupled.
fn classify_approval(env: &AgentEnvelope) -> Option<Alert> {
    let obj = env.body.as_object()?;
    if obj.get("kind").and_then(|v| v.as_str()) != Some("awaiting_approval") {
        return None;
    }
    let approval_id = obj.get("approval_id").and_then(|v| v.as_str())?.to_string();
    let capability = obj.get("capability").and_then(|v| v.as_str()).unwrap_or("?");
    let target = obj.get("target_ref").and_then(|v| v.as_str()).unwrap_or("-");
    let actor = obj.get("actor").and_then(|v| v.as_str()).unwrap_or("?");
    let body = AlertBody {
        title: "🔐 approval required".to_string(),
        body: format!(
            "{capability} on {target} (requested by {actor})\nReply `approve {approval_id}` or `deny {approval_id}` to decide.",
        ),
    };
    Some(Alert {
        class: "approval",
        severity: "default".to_string(),
        signature: approval_id,
        body,
    })
}

/// A tiny TTL admit cache (fixed window). `admit(sig)` returns true the first
/// time a signature is seen within the window, false while it is still warm.
struct DedupCache {
    seen: Mutex<HashMap<String, Instant>>,
    ttl: Duration,
}

impl DedupCache {
    fn new(ttl: Duration) -> Self {
        Self { seen: Mutex::new(HashMap::new()), ttl }
    }
    fn admit(&self, sig: &str) -> bool {
        let now = Instant::now();
        let mut g = self.seen.lock().unwrap();
        // opportunistic eviction of expired entries
        g.retain(|_, t| now.duration_since(*t) < self.ttl);
        if let Some(t) = g.get(sig) {
            if now.duration_since(*t) < self.ttl {
                return false;
            }
        }
        g.insert(sig.to_string(), now);
        true
    }
}

/// Spawn the alert router. No-op (with a log) when no channel is enabled — the
/// router never fires against an empty registry.
pub fn spawn_alert_router(
    state: AppState,
    mut rx: tokio::sync::broadcast::Receiver<AgentEnvelope>,
) {
    if state.gateways.is_empty() {
        tracing::info!("alert router: no channel enabled — outbound routing disabled");
        return;
    }
    tokio::spawn(async move {
        let dedup = DedupCache::new(DEDUP_TTL);
        tracing::info!(
            channels = ?state.gateways.enabled_channels(),
            "alert router spawned (bus subscriber)"
        );
        loop {
            match rx.recv().await {
                Ok(env) => {
                    if let Some(alert) = classify(&env) {
                        if dedup.admit(&alert.signature) {
                            route(&state, &alert).await;
                        } else {
                            tracing::debug!(sig = %alert.signature, "alert deduped (within TTL)");
                        }
                    }
                }
                // A slow router lagged behind the broadcast — skip the gap, keep
                // going (notification is a non-critical plane, FR-GW-15).
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "alert router lagged the bus");
                }
                Err(RecvError::Closed) => {
                    tracing::info!("alert router: bus closed, exiting");
                    break;
                }
            }
        }
    });
}

/// Resolve recipients + deliver, fail-soft, recording every attempt.
async fn route(state: &AppState, alert: &Alert) {
    let recipients = resolve_recipients(state, alert.class, &alert.severity).await;
    if recipients.is_empty() {
        tracing::warn!(
            class = alert.class,
            severity = %alert.severity,
            "alert has no enrolled recipient (fail-closed) — not delivered"
        );
        return;
    }
    for r in recipients {
        let Some(adapter) = state.gateways.get(&r.channel) else {
            tracing::warn!(channel = %r.channel, "alert recipient channel not enabled — skipped");
            continue;
        };
        let (status, detail) = match adapter.deliver_alert(&r, &alert.body).await {
            Ok(()) => ("delivered", None),
            // FR-GW-15: log + record, never propagate — the originating loop is
            // already done; a down channel must not stall anything.
            Err(e) => {
                tracing::warn!(channel = %r.channel, error = %e, "alert delivery failed (fail-soft)");
                ("failed", Some(e.to_string()))
            }
        };
        record_delivery(state, alert, &r, status, detail.as_deref()).await;
    }
}

/// Resolve the routing rule `channels.alerts.<class>.<severity>` (fallback
/// `channels.alerts.default`) to a set of ENROLLED, ACTIVE recipients. The rule
/// value is a comma-separated list of `channel:handle`; each handle must map to
/// a bound, active gateway identity (fail-closed, FR-GW-16) or it is dropped.
async fn resolve_recipients(state: &AppState, class: &str, severity: &str) -> Vec<Recipient> {
    let cfg = state.config.current();
    let key = format!("channels.alerts.{class}.{severity}");
    let raw = cfg
        .opt_string(&key, None)
        .or_else(|| cfg.opt_string("channels.alerts.default", None));
    let Some(raw) = raw else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let Some((channel, handle)) = entry.split_once(':') else {
            tracing::warn!(entry, "alert routing entry is not `channel:handle` — skipped");
            continue;
        };
        let (channel, handle) = (channel.trim().to_string(), handle.trim().to_string());
        match is_enrolled_active(state, &channel, &handle).await {
            Ok(true) => out.push(Recipient { channel, to: handle }),
            Ok(false) => tracing::warn!(
                channel = %channel,
                handle = %handle,
                "alert recipient is not an enrolled active identity — dropped (fail-closed)"
            ),
            Err(e) => tracing::warn!(error = %e, "enrolment check failed — recipient dropped"),
        }
    }
    out
}

/// True iff `(channel, handle)` is a bound gateway identity of an ACTIVE user.
async fn is_enrolled_active(
    state: &AppState,
    channel: &str,
    handle: &str,
) -> Result<bool, String> {
    let client = state.db.get().await.map_err(|e| e.to_string())?;
    let row = client
        .query_opt(
            "SELECT 1
               FROM public.gateway_identities gi
               JOIN public.users u ON u.id = gi.user_id
              WHERE gi.channel = $1 AND gi.platform_handle = $2 AND u.status = 'active'",
            &[&channel, &handle],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.is_some())
}

/// Record one delivery attempt in `alert_deliveries` (fail-soft evidence trail).
/// A record failure is itself logged-and-swallowed — bookkeeping must never
/// escalate into the originating loop.
async fn record_delivery(
    state: &AppState,
    alert: &Alert,
    r: &Recipient,
    status: &str,
    detail: Option<&str>,
) {
    let client = match state.db.get().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "alert_deliveries: pool unavailable");
            return;
        }
    };
    let severity = if alert.severity == "default" { None } else { Some(alert.severity.clone()) };
    if let Err(e) = client
        .execute(
            "INSERT INTO public.alert_deliveries
               (alert_class, severity, signature, channel, recipient, status, detail)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &alert.class,
                &severity,
                &alert.signature,
                &r.channel,
                &r.to,
                &status,
                &detail,
            ],
        )
        .await
    {
        tracing::warn!(error = %e, "alert_deliveries insert failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_admits_once_per_window() {
        let d = DedupCache::new(Duration::from_secs(60));
        assert!(d.admit("sig-1"));
        assert!(!d.admit("sig-1")); // within window
        assert!(d.admit("sig-2")); // distinct signature
    }

    #[test]
    fn classify_anomaly_from_envelope_body() {
        use daimon_core::{AgentEnvelope, AgentId, AuditMetadata, Recipient as BusRecipient};
        let anomaly = serde_json::json!({
            "anomaly_id": "00000000-0000-0000-0000-000000000001",
            "source": "prometheus",
            "source_id": "node-1",
            "severity": "critical",
            "title": "CPU saturation > 90%",
            "metric_name": "node.cpu.saturation_pct",
            "metric_value": 95.0,
            "threshold": 90.0,
            "query": "cpu_sat",
            "target_ref": "target://k3s-lab"
        });
        let env = AgentEnvelope {
            correlation_id: uuid::Uuid::nil(),
            from: AgentId::new("observer"),
            to: BusRecipient::ByCapability {
                name: "harness.triage.anomaly".into(),
                version_req: semver::VersionReq::parse("^1").unwrap(),
            },
            reply_to: None,
            body: anomaly,
            audit: AuditMetadata::default(),
        };
        let alert = classify(&env).expect("should classify as anomaly");
        assert_eq!(alert.class, "anomaly");
        assert_eq!(alert.severity, "critical");
        assert_eq!(alert.signature, "00000000-0000-0000-0000-000000000001");
        assert!(alert.body.body.contains("target://k3s-lab"));
    }

    #[test]
    fn classify_approval_from_envelope_body() {
        use daimon_core::{AgentEnvelope, AgentId, AuditMetadata, Recipient as BusRecipient};
        let env = AgentEnvelope {
            correlation_id: uuid::Uuid::nil(),
            from: AgentId::new("guard"),
            to: BusRecipient::ByCapability {
                name: "harness.alert.approval".into(),
                version_req: semver::VersionReq::parse("^1").unwrap(),
            },
            reply_to: None,
            body: serde_json::json!({
                "kind": "awaiting_approval",
                "approval_id": "abc-123",
                "capability": "orchestrator.k8s.deploy.restart",
                "target_ref": "target://k3s-lab",
                "actor": "gw:telegram:66784431"
            }),
            audit: AuditMetadata::default(),
        };
        let alert = classify(&env).expect("should classify as approval");
        assert_eq!(alert.class, "approval");
        assert_eq!(alert.signature, "abc-123");
        assert!(alert.body.body.contains("approve abc-123"));
    }
}
