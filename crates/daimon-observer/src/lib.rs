//! Observer tier (Phase 7 D4).
//!
//! Two ingestion paths:
//! 1. **Prometheus pull** — query `/api/v1/query` against a Prometheus
//!    endpoint, mapped through a `NamedQueryLibrary` (operator-curated
//!    PromQL) on a fixed cadence.
//! 2. **Push** — agents and platform pollers call `MetricSink::push` directly
//!    to record their own telemetry (token usage, request latency, plan
//!    durations, etc.).
//!
//! Both paths write through a `MetricSink`. Phase 8 lock per MASTERPLAN
//! §3.5: default sink is `VictoriaMetricsSink` (Prom-text-format POST to
//! VM's `/api/v1/import/prometheus`). `PostgresMetricSink` is retained for
//! tests + grandfathering; production deployments after V015 do not write
//! to `observer.metrics`. Threshold-bound named queries emit
//! `AnomalyDetected` rows into `observer.anomalies` + (Phase 8) emit a bus
//! envelope subscribed by Guard + Orchestrator.

pub mod error;
pub mod ingest;
pub mod prometheus;
pub mod queries;
pub mod sink;
pub mod vm_sink;

pub use error::{Error, Result};
pub use ingest::{ObserverIngest, ObserverIngestConfig};
pub use prometheus::PrometheusClient;
pub use queries::{NamedQuery, NamedQueryLibrary, Threshold};
pub use sink::{MetricPoint, MetricSink, PostgresMetricSink};
pub use vm_sink::VictoriaMetricsSink;
