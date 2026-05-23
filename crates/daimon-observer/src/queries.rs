//! Operator-curated PromQL library. Phase 7 ships a small default set;
//! the catalog is extensible at runtime.
//!
//! Thresholds are simple: greater-than / less-than comparison against the
//! returned value. Cross-rule logic (multi-metric anomaly correlation) lands
//! in Phase 7.1+.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdOp {
    Gt,
    Gte,
    Lt,
    Lte,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Threshold {
    pub op: ThresholdOp,
    pub value: f64,
    pub severity: String,
    pub title: String,
}

impl Threshold {
    pub fn breaches(&self, v: f64) -> bool {
        match self.op {
            ThresholdOp::Gt => v > self.value,
            ThresholdOp::Gte => v >= self.value,
            ThresholdOp::Lt => v < self.value,
            ThresholdOp::Lte => v <= self.value,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedQuery {
    pub name: String,
    pub promql: String,
    pub source: String,
    /// `source_id` template — `{instance}` is substituted from the metric
    /// labels. Caller-friendly default: just the literal source_id.
    #[serde(default)]
    pub source_id_label: Option<String>,
    /// Dotted metric name to record under in observer.metrics.
    pub metric_name: String,
    #[serde(default)]
    pub thresholds: Vec<Threshold>,
}

#[derive(Debug, Clone, Default)]
pub struct NamedQueryLibrary {
    pub queries: Vec<NamedQuery>,
}

impl NamedQueryLibrary {
    pub fn new(queries: Vec<NamedQuery>) -> Self {
        Self { queries }
    }

    pub fn default_library() -> Self {
        // Curated defaults — operator can extend at runtime.
        let queries = vec![
            NamedQuery {
                name: "node_cpu_saturation".into(),
                promql:
                    "100 - (avg by (instance)(rate(node_cpu_seconds_total{mode=\"idle\"}[5m])) * 100)"
                        .into(),
                source: "prometheus".into(),
                source_id_label: Some("instance".into()),
                metric_name: "node.cpu.saturation_pct".into(),
                thresholds: vec![
                    Threshold {
                        op: ThresholdOp::Gt,
                        value: 90.0,
                        severity: "warning".into(),
                        title: "CPU saturation > 90%".into(),
                    },
                    Threshold {
                        op: ThresholdOp::Gt,
                        value: 98.0,
                        severity: "critical".into(),
                        title: "CPU saturation > 98%".into(),
                    },
                ],
            },
            NamedQuery {
                name: "node_memory_pressure".into(),
                promql:
                    "(1 - (node_memory_MemAvailable_bytes / node_memory_MemTotal_bytes)) * 100"
                        .into(),
                source: "prometheus".into(),
                source_id_label: Some("instance".into()),
                metric_name: "node.memory.pressure_pct".into(),
                thresholds: vec![Threshold {
                    op: ThresholdOp::Gt,
                    value: 90.0,
                    severity: "warning".into(),
                    title: "Memory pressure > 90%".into(),
                }],
            },
            NamedQuery {
                name: "node_disk_full".into(),
                promql:
                    "100 - ((node_filesystem_avail_bytes{fstype!~\"tmpfs|overlay\"} * 100) / node_filesystem_size_bytes{fstype!~\"tmpfs|overlay\"})"
                        .into(),
                source: "prometheus".into(),
                source_id_label: Some("instance".into()),
                metric_name: "node.disk.used_pct".into(),
                thresholds: vec![Threshold {
                    op: ThresholdOp::Gt,
                    value: 85.0,
                    severity: "warning".into(),
                    title: "Disk usage > 85%".into(),
                }],
            },
            NamedQuery {
                name: "node_nic_errors".into(),
                promql: "rate(node_network_receive_errs_total[5m]) + rate(node_network_transmit_errs_total[5m])".into(),
                source: "prometheus".into(),
                source_id_label: Some("instance".into()),
                metric_name: "node.nic.error_rate".into(),
                thresholds: vec![Threshold {
                    op: ThresholdOp::Gt,
                    value: 1.0,
                    severity: "warning".into(),
                    title: "NIC error rate > 1/s".into(),
                }],
            },
        ];
        Self { queries }
    }
}
