//! The bus event the observer emits when a named-query threshold breaches
//! (P3 commit 7, AC-P3-01).
//!
//! [`AnomalyDetected`] is the payload of the `AgentEnvelope` the observer sends
//! `ByCapability` `"harness.triage.anomaly"` after it persists a row into
//! `observer.anomalies`. It is the FIRST hop of the AIOps loop: detect →
//! triage. The `anomaly_id` is the real `observer.anomalies` row id (the INSERT
//! uses `RETURNING id`), so a consumer can join the event back to the durable
//! record.
//!
//! This type derives `Serialize`/`Deserialize` because it travels as
//! `serde_json::Value` inside the envelope body — the TriageAgent decodes it on
//! the other side.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A threshold-breach anomaly, carried on the bus to the triage tier.
///
/// The fields mirror the durable `observer.anomalies` row plus the query that
/// produced it, so triage has everything it needs to open a plan without
/// re-querying: what breached (`metric_name`/`metric_value`), the bound it
/// crossed (`threshold`), where (`source`/`source_id`/`target_ref`), and the
/// PromQL (`query`) for provenance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnomalyDetected {
    /// The `observer.anomalies` row id (from the INSERT ... RETURNING id).
    pub anomaly_id: Uuid,
    /// Metric source system, e.g. `"prometheus"`.
    pub source: String,
    /// The instance/target the metric was scoped to (from the query's
    /// `source_id_label`, e.g. a Prometheus `instance` label), or `"default"`.
    pub source_id: String,
    /// Threshold severity, e.g. `"warning"` / `"critical"`.
    pub severity: String,
    /// Human-readable breach title, e.g. `"CPU saturation > 90%"`.
    pub title: String,
    /// Dotted metric name, e.g. `"node.cpu.saturation_pct"`.
    pub metric_name: String,
    /// The observed value that breached.
    pub metric_value: f64,
    /// The bound that was crossed.
    pub threshold: f64,
    /// The named query (its `name`) that produced this anomaly — provenance.
    pub query: String,
    /// A best-effort `target://…` ref derived from `source_id`, when the
    /// source_id looks like a target the driver tier could address. `None`
    /// when no convention applies (triage then opens a context-only plan).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_ref: Option<String>,
}

impl AnomalyDetected {
    /// The capability name the observer addresses this event to
    /// (`Recipient::ByCapability`). The TriageAgent advertises EXACTLY this
    /// capability, which is what routes the envelope to it (the load-bearing
    /// routing contract, AC-P3-01).
    pub const TRIAGE_CAPABILITY: &'static str = "harness.triage.anomaly";
}
