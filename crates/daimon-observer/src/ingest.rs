//! Observer ingest loop — runs the named-query library against a
//! Prometheus endpoint on a cadence, writes metrics, raises anomalies.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use daimon_db::Pool;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::prometheus::PrometheusClient;
use crate::queries::NamedQueryLibrary;
use crate::sink::{MetricPoint, MetricSink, PostgresMetricSink};

#[derive(Debug, Clone)]
pub struct ObserverIngestConfig {
    pub tenant_id: Uuid,
    pub prom_url: String,
    pub interval: Duration,
}

impl Default for ObserverIngestConfig {
    fn default() -> Self {
        Self {
            tenant_id: Uuid::nil(),
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
}

impl ObserverIngest {
    pub fn new(
        cfg: ObserverIngestConfig,
        pool: Pool,
        library: NamedQueryLibrary,
    ) -> Result<Self> {
        let prom = PrometheusClient::new(&cfg.prom_url)?;
        let sink = Arc::new(PostgresMetricSink::new(pool.clone()));
        Ok(Self {
            cfg,
            prom,
            sink,
            pool,
            library,
        })
    }

    /// Spawn the background loop. Returns immediately. Loop runs forever
    /// until the runtime exits.
    pub fn spawn(self) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(self.cfg.interval);
            info!(
                tenant = %self.cfg.tenant_id,
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
                            .raise_anomaly(self.cfg.tenant_id, q, t, &source_id, s.value)
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
            self.sink.push_batch(self.cfg.tenant_id, points).await?;
        }
        if anomalies_raised > 0 {
            info!(raised = anomalies_raised, "anomalies emitted");
        }
        Ok(())
    }

    async fn raise_anomaly(
        &self,
        tenant_id: Uuid,
        q: &crate::queries::NamedQuery,
        t: &crate::queries::Threshold,
        source_id: &str,
        value: f64,
    ) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "INSERT INTO observer.anomalies
                    (tenant_id, source, source_id, severity, title, description,
                     metric_name, metric_value, threshold, metadata)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                &[
                    &tenant_id,
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
        Ok(())
    }
}
