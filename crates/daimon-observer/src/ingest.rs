//! Observer ingest loop — runs the named-query library against a
//! Prometheus endpoint on a cadence, writes metrics, raises anomalies.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use daimon_core::{AgentEnvelope, AgentId, BusHandle, Recipient};
use daimon_db::Pool;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::event::AnomalyDetected;
use crate::prometheus::PrometheusClient;
use crate::queries::NamedQueryLibrary;
use crate::sink::{MetricPoint, MetricSink};

#[derive(Debug, Clone)]
pub struct ObserverIngestConfig {
    pub prom_url: String,
    pub interval: Duration,
}

impl Default for ObserverIngestConfig {
    fn default() -> Self {
        Self {
            prom_url: "http://localhost:9090".into(),
            interval: Duration::from_secs(30),
        }
    }
}

pub struct ObserverIngest {
    cfg: ObserverIngestConfig,
    prom: PrometheusClient,
    sink: Arc<dyn MetricSink>,
    pool: Pool,
    library: NamedQueryLibrary,
    /// P3 commit 7: the agent bus, if wired. When present, a persisted anomaly
    /// also emits an `AnomalyDetected` envelope `ByCapability`
    /// `"harness.triage.anomaly"` (fire-and-forget). `None` (the `new()`
    /// default) keeps the pre-P3 behaviour: persist-only, no emit. This is an
    /// abstract `Arc<dyn BusHandle>` — the observer never holds the concrete
    /// `InProcBus`, so it never depends on `daimon-runtime` (D21).
    bus: Option<Arc<dyn BusHandle>>,
}

impl ObserverIngest {
    pub fn new(
        cfg: ObserverIngestConfig,
        sink: Arc<dyn MetricSink>,
        pool: Pool,
        library: NamedQueryLibrary,
    ) -> Result<Self> {
        let prom = PrometheusClient::new(&cfg.prom_url)?;
        Ok(Self {
            cfg,
            prom,
            sink,
            pool,
            library,
            bus: None,
        })
    }

    /// Wire the agent bus so persisted anomalies also emit an `AnomalyDetected`
    /// envelope for the triage tier. Fluent so boot can chain
    /// `.with_bus(bus.handle()).spawn()`.
    pub fn with_bus(mut self, bus: Arc<dyn BusHandle>) -> Self {
        self.bus = Some(bus);
        self
    }

    /// Spawn the background loop. Returns immediately. Loop runs forever
    /// until the runtime exits.
    pub fn spawn(self) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(self.cfg.interval);
            info!(
                prom = %self.cfg.prom_url,
                queries = self.library.queries.len(),
                "observer ingest spawned"
            );
            loop {
                interval.tick().await;
                if let Err(e) = self.run_once().await {
                    error!(error = %e, "observer ingest cycle failed");
                }
            }
        });
    }

    #[instrument(skip(self))]
    async fn run_once(&self) -> Result<()> {
        let now = Utc::now();
        let mut points: Vec<MetricPoint> = Vec::new();
        let mut anomalies_raised = 0;

        for q in &self.library.queries {
            let samples = match self.prom.instant(&q.promql).await {
                Ok(s) => s,
                Err(e) => {
                    warn!(query = %q.name, error = %e, "instant query failed");
                    continue;
                }
            };
            for s in samples {
                let source_id = q
                    .source_id_label
                    .as_ref()
                    .and_then(|lbl| s.metric.get(lbl).and_then(|v| v.as_str()))
                    .unwrap_or("default")
                    .to_string();
                points.push(MetricPoint {
                    ts: now,
                    source: q.source.clone(),
                    source_id: source_id.clone(),
                    name: q.metric_name.clone(),
                    value: s.value,
                    labels: s.metric.clone(),
                });
                for t in &q.thresholds {
                    if t.breaches(s.value) {
                        if let Err(e) = self
                            .raise_anomaly(q, t, &source_id, s.value)
                            .await
                        {
                            warn!(error = %e, "anomaly insert failed");
                        } else {
                            anomalies_raised += 1;
                        }
                        break; // one anomaly per query+sample, highest-severity wins
                    }
                }
            }
        }

        if !points.is_empty() {
            self.sink.push_batch(points).await?;
        }
        if anomalies_raised > 0 {
            info!(raised = anomalies_raised, "anomalies emitted");
        }
        Ok(())
    }

    async fn raise_anomaly(
        &self,
        q: &crate::queries::NamedQuery,
        t: &crate::queries::Threshold,
        source_id: &str,
        value: f64,
    ) -> Result<()> {
        let client = self.pool.get().await?;
        // RETURNING id so the emitted event carries the REAL durable row id —
        // a triage consumer can join the bus event back to observer.anomalies.
        let row = client
            .query_one(
                "INSERT INTO observer.anomalies
                    (source, source_id, severity, title, description,
                     metric_name, metric_value, threshold, metadata)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 RETURNING id",
                &[
                    &q.source,
                    &source_id,
                    &t.severity,
                    &t.title,
                    &Some(format!("PromQL: {}", q.promql)),
                    &Some(q.metric_name.clone()),
                    &value,
                    &t.value,
                    &serde_json::json!({"query": q.name}),
                ],
            )
            .await?;
        let anomaly_id: Uuid = row.get(0);

        // P3 commit 7 — emit onto the bus (fire-and-forget). This runs AFTER a
        // successful persist, so the durable record is truth and the bus event
        // is the aid. A zero-subscriber `send` is a silent no-op on `InProcBus`,
        // so persistence is NEVER blocked by triage being absent.
        if let Some(bus) = &self.bus {
            let event = AnomalyDetected {
                anomaly_id,
                source: q.source.clone(),
                source_id: source_id.to_string(),
                severity: t.severity.clone(),
                title: t.title.clone(),
                metric_name: q.metric_name.clone(),
                metric_value: value,
                threshold: t.value,
                query: q.name.clone(),
                target_ref: target_ref_from_source_id(source_id),
            };
            match anomaly_envelope(&event) {
                Ok(env) => {
                    // fire-and-forget: an emit failure must not fail the cycle
                    // (the anomaly is already persisted). Log and move on.
                    if let Err(e) = bus.send(env).await {
                        warn!(error = %e, anomaly_id = %anomaly_id, "anomaly bus emit failed");
                    }
                }
                Err(e) => {
                    warn!(error = %e, anomaly_id = %anomaly_id, "anomaly envelope build failed");
                }
            }
        }
        Ok(())
    }
}

/// Build the `AnomalyDetected` bus envelope, addressed `ByCapability`
/// `"harness.triage.anomaly"` `^1`. Extracted (pool-free) so the routing +
/// body contract is unit-testable against an `InProcBus` without a live DB.
pub(crate) fn anomaly_envelope(event: &AnomalyDetected) -> Result<AgentEnvelope> {
    let body = serde_json::to_value(event).map_err(|e| Error::Decode(e.to_string()))?;
    let version_req = "^1"
        .parse()
        .map_err(|e| Error::Other(format!("bad triage version_req: {e}")))?;
    Ok(AgentEnvelope::new(
        AgentId::new("observer"),
        Recipient::ByCapability {
            name: AnomalyDetected::TRIAGE_CAPABILITY.to_string(),
            version_req,
        },
        body,
    ))
}

/// Best-effort `target://…` derivation from an anomaly's `source_id`.
///
/// Convention: a Prometheus `instance` label is usually `host:port`. We take
/// the host part and mint a `target://<host>` ref so the triage tier has a
/// candidate target to address a read step at. `"default"` (the no-label
/// sentinel) yields `None` — there is nothing addressable, so triage opens a
/// context-only plan. This is deliberately conservative: a bad target ref is
/// worse than none (triage falls back cleanly on `None`).
fn target_ref_from_source_id(source_id: &str) -> Option<String> {
    if source_id.is_empty() || source_id == "default" {
        return None;
    }
    let host = source_id.split(':').next().unwrap_or(source_id);
    if host.is_empty() {
        return None;
    }
    Some(format!("target://{host}"))
}

#[cfg(test)]
mod tests {
    //! P3 commit 7 tests. `raise_anomaly` itself needs a live Postgres pool
    //! (covered by DB-integration coverage elsewhere), but the LOAD-BEARING
    //! contract is the emit: a breaching sample produces an `AnomalyDetected`
    //! envelope addressed `ByCapability` `"harness.triage.anomaly"` `^1`. We
    //! exercise exactly that seam (`anomaly_envelope`) by sending it over a
    //! `MockBus` — an in-crate `BusHandle` capturing sent envelopes, so the
    //! observer stays free of `daimon-runtime` (D21).

    use super::*;
    use async_trait::async_trait;
    use daimon_core::CoreError;
    use tokio::sync::mpsc;

    /// Minimal `BusHandle` that captures every sent envelope. Stands in for the
    /// runtime's `InProcBus` without depending on `daimon-runtime`.
    struct MockBus {
        tx: mpsc::UnboundedSender<AgentEnvelope>,
    }

    #[async_trait]
    impl BusHandle for MockBus {
        async fn send(&self, env: AgentEnvelope) -> std::result::Result<(), CoreError> {
            let _ = self.tx.send(env);
            Ok(())
        }
    }

    fn sample_event(source_id: &str) -> AnomalyDetected {
        AnomalyDetected {
            anomaly_id: Uuid::new_v4(),
            source: "prometheus".into(),
            source_id: source_id.into(),
            severity: "critical".into(),
            title: "CPU saturation > 98%".into(),
            metric_name: "node.cpu.saturation_pct".into(),
            metric_value: 99.2,
            threshold: 98.0,
            query: "node_cpu_saturation".into(),
            target_ref: target_ref_from_source_id(source_id),
        }
    }

    #[tokio::test]
    async fn emits_anomaly_detected_by_capability_to_triage() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let bus: Arc<dyn BusHandle> = Arc::new(MockBus { tx });

        // The exact seam raise_anomaly drives after a successful persist.
        let event = sample_event("node-01:9100");
        let env = anomaly_envelope(&event).expect("build envelope");
        bus.send(env).await.expect("send");

        let published = rx.recv().await.expect("an envelope was published");

        // Routed ByCapability to the triage capability, at ^1.
        match &published.to {
            Recipient::ByCapability { name, version_req } => {
                assert_eq!(name, "harness.triage.anomaly");
                assert!(version_req.matches(&semver::Version::new(1, 0, 0)));
            }
            other => panic!("expected ByCapability, got {other:?}"),
        }
        assert_eq!(published.from, AgentId::new("observer"));

        // The body round-trips back into the same AnomalyDetected.
        let decoded: AnomalyDetected =
            serde_json::from_value(published.body).expect("decode body");
        assert_eq!(decoded, event);
        assert_eq!(decoded.target_ref.as_deref(), Some("target://node-01"));
    }

    #[test]
    fn target_ref_convention() {
        assert_eq!(
            target_ref_from_source_id("node-01:9100").as_deref(),
            Some("target://node-01")
        );
        assert_eq!(
            target_ref_from_source_id("mikrotik-edge").as_deref(),
            Some("target://mikrotik-edge")
        );
        // The no-label sentinel and empties yield None (context-only plan).
        assert_eq!(target_ref_from_source_id("default"), None);
        assert_eq!(target_ref_from_source_id(""), None);
    }
}
