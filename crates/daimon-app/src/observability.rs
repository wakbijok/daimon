//! P3 commit 11 — self-observability surface (AC-P3-06).
//!
//! daimon must be able to observe *itself*, not just the fleet it watches
//! (M-observer-cant-self-observe). Three additive pieces land in P3:
//!
//! 1. A `tracing` subscriber installed once at the top of `main` (in `main.rs`)
//!    so the broker/guard/transport/observer `#[instrument]`/`info!`/`warn!`
//!    spans — dropped today for lack of any subscriber — actually surface.
//! 2. The `/healthz` + `/metrics` routes below, mounted on the axum Router
//!    BEFORE the Leptos routes (mirroring the `/api/v1/ws` route). Both are
//!    UNAUTHENTICATED: they are an infra surface meant to sit behind the
//!    reverse proxy / systemd probe, distinct from the authed `/api/v1/ws`.
//! 3. [`SelfMetrics`] — a hand-rolled `AtomicU64` counter set rendered as
//!    Prometheus text by hand. We deliberately do NOT pull the `prometheus`
//!    crate: it drags in protobuf, and the whole reason the memory tier is a
//!    sidecar (not embedded) is musl-static binary size. A handful of counters
//!    does not justify that weight.
//!
//! This whole module is `ssr`-gated — it is server-only and must never enter
//! the wasm/hydrate bundle.
#![cfg(feature = "ssr")]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

/// Hand-rolled self-metrics — monotonic `u64` counters, incremented at real
/// call sites across the app + observer, rendered as Prometheus exposition text
/// by [`SelfMetrics::render_prometheus`].
///
/// Held as `Arc<SelfMetrics>` in [`AppState`]. The three observer-owned counters
/// are `Arc<AtomicU64>` fields (not inline `AtomicU64`) on purpose: `main.rs`
/// clones those three Arcs and hands them to `ObserverIngest::with_metrics`, so
/// the observer increments the SAME atomics the `/metrics` renderer reads — one
/// source of truth — WITHOUT `daimon-observer` ever depending on this app-side
/// type (it takes bare `Arc<AtomicU64>` via `std::sync::atomic` alone).
///
/// The remaining counters are app-local, so they stay inline `AtomicU64`.
#[derive(Debug)]
pub struct SelfMetrics {
    /// Observer ingest cycles completed (one per `run_once`). Shared with the
    /// observer via a cloned `Arc` handle.
    pub ingest_cycles: Arc<AtomicU64>,
    /// Anomalies persisted+emitted by the observer. Shared handle.
    pub anomalies_raised: Arc<AtomicU64>,
    /// Metric-sink `push_batch` failures in the observer. Shared handle.
    pub sink_push_failures: Arc<AtomicU64>,
    /// Orchestrator plan dispatches. Reserved: the orchestrator dispatch path is
    /// not cheaply reachable from the app without a cross-crate coupling the
    /// plan explicitly said to skip, so this stays 0 for now (the series is
    /// still exported so scrapers see a stable schema).
    pub plan_dispatches: AtomicU64,
    /// Chat pre-turn recalls that came back degraded (memory unreachable/slow).
    /// Incremented in `chat.rs` on the degraded path.
    pub memory_recall_degraded: AtomicU64,
}

impl Default for SelfMetrics {
    fn default() -> Self {
        Self {
            ingest_cycles: Arc::new(AtomicU64::new(0)),
            anomalies_raised: Arc::new(AtomicU64::new(0)),
            sink_push_failures: Arc::new(AtomicU64::new(0)),
            plan_dispatches: AtomicU64::new(0),
            memory_recall_degraded: AtomicU64::new(0),
        }
    }
}

impl SelfMetrics {
    /// A fresh zeroed counter set.
    pub fn new() -> Self {
        Self::default()
    }

    /// The three observer-owned counter handles, in the order
    /// `ObserverIngest::with_metrics(ingest, anomalies, failures)` expects.
    /// These are clones of the SAME `Arc<AtomicU64>` the renderer reads, so an
    /// observer increment is immediately visible on `/metrics`.
    pub fn observer_handles(&self) -> (Arc<AtomicU64>, Arc<AtomicU64>, Arc<AtomicU64>) {
        (
            self.ingest_cycles.clone(),
            self.anomalies_raised.clone(),
            self.sink_push_failures.clone(),
        )
    }

    /// Increment the degraded-recall counter (called from the chat hot path).
    pub fn inc_recall_degraded(&self) {
        self.memory_recall_degraded.fetch_add(1, Ordering::Relaxed);
    }

    /// Render the current counter values as Prometheus exposition text.
    ///
    /// Hand-rolled: `# HELP` + `# TYPE … counter` + one sample line per series,
    /// named `daimon_<name>_total`. No labels (single-process, single-org). This
    /// is the entire reason we can avoid the `prometheus`/protobuf dependency.
    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(1024);
        for (name, help, value) in [
            (
                "daimon_ingest_cycles_total",
                "Observer ingest cycles completed.",
                self.ingest_cycles.load(Ordering::Relaxed),
            ),
            (
                "daimon_anomalies_raised_total",
                "Anomalies persisted and emitted by the observer.",
                self.anomalies_raised.load(Ordering::Relaxed),
            ),
            (
                "daimon_sink_push_failures_total",
                "Metric-sink push_batch failures in the observer.",
                self.sink_push_failures.load(Ordering::Relaxed),
            ),
            (
                "daimon_plan_dispatches_total",
                "Orchestrator plan dispatches.",
                self.plan_dispatches.load(Ordering::Relaxed),
            ),
            (
                "daimon_memory_recall_degraded_total",
                "Chat pre-turn recalls that returned degraded.",
                self.memory_recall_degraded.load(Ordering::Relaxed),
            ),
        ] {
            out.push_str(&format!("# HELP {name} {help}\n"));
            out.push_str(&format!("# TYPE {name} counter\n"));
            out.push_str(&format!("{name} {value}\n"));
        }
        out
    }
}

/// `GET /healthz` — unauthenticated liveness/readiness probe.
///
/// Returns 200 whenever Postgres is reachable (the hard dependency). The memory
/// tier is non-critical — an unreachable sidecar degrades recall but daimon
/// stays up — so it is reported as `degraded` in the body but does NOT flip the
/// status code. Only a Postgres failure yields 503.
pub async fn healthz(Extension(state): Extension<AppState>) -> Response {
    // DB probe: acquiring a pooled connection is the cheapest real reachability
    // check (deadpool validates on checkout).
    let db_ok = state.db.get().await.is_ok();

    // Memory probe: non-blocking, non-critical. `health()` never panics; the
    // NullMemory impl simply reports unreachable.
    let mem_reachable = state.memory.health().await.reachable;

    let body = serde_json::json!({
        "db": if db_ok { "ok" } else { "err" },
        "memory": if mem_reachable { "ok" } else { "degraded" },
    });

    let code = if db_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, axum::Json(body)).into_response()
}

/// `GET /metrics` — unauthenticated Prometheus exposition of daimon's OWN
/// self-metrics (distinct from the fleet metrics the observer scrapes). Text
/// format, hand-rendered, no `prometheus` crate.
pub async fn metrics(Extension(state): Extension<AppState>) -> Response {
    let body = state.self_metrics.render_prometheus();
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_contains_all_series_and_types() {
        let m = SelfMetrics::new();
        m.ingest_cycles.store(3, Ordering::Relaxed);
        m.anomalies_raised.store(1, Ordering::Relaxed);
        m.inc_recall_degraded();
        let text = m.render_prometheus();

        // The AC-P3-06 verify anchor.
        assert!(text.contains("daimon_ingest_cycles_total 3"));
        assert!(text.contains("daimon_anomalies_raised_total 1"));
        assert!(text.contains("daimon_memory_recall_degraded_total 1"));
        // TYPE lines present for the Prometheus text format.
        assert!(text.contains("# TYPE daimon_ingest_cycles_total counter"));
        assert!(text.contains("daimon_sink_push_failures_total 0"));
        assert!(text.contains("daimon_plan_dispatches_total 0"));
    }

    #[test]
    fn observer_handles_share_the_same_atomics() {
        let m = SelfMetrics::new();
        let (ingest, anomalies, failures) = m.observer_handles();
        // An increment via the shared handle is visible through the renderer.
        ingest.fetch_add(2, Ordering::Relaxed);
        anomalies.fetch_add(5, Ordering::Relaxed);
        failures.fetch_add(1, Ordering::Relaxed);
        let text = m.render_prometheus();
        assert!(text.contains("daimon_ingest_cycles_total 2"));
        assert!(text.contains("daimon_anomalies_raised_total 5"));
        assert!(text.contains("daimon_sink_push_failures_total 1"));
    }
}
