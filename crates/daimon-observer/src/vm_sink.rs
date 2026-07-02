//! VictoriaMetrics sink — ingests via VM's `/api/v1/import/prometheus`
//! endpoint (Prometheus exposition format).
//!
//! Phase 8 storage lock per MASTERPLAN §3.5: VM replaces the Postgres
//! `observer.metrics` table as the time-series tier (#4). The Prom-text
//! ingest path is chosen over `remote_write` (protobuf+snappy) for
//! simplicity — adequate for daimon's ingest volumes through Phase 8
//! demo; the remote_write upgrade is a phase 8.1 task only if measured
//! throughput requires it.
//!
//! Encoding rules:
//! - `MetricPoint.name` is dotted (e.g. `pve.node.cpu_pct`). VM (like
//!   Prometheus) requires `[a-zA-Z_][a-zA-Z0-9_]*` — dots become
//!   underscores: `pve_node_cpu_pct`.
//! - `source` + `source_id` become labels.
//! - `MetricPoint.labels` (JSON object) flattens into Prom labels. Nested
//!   structure is collapsed via `serde_json::to_string` for non-scalars.
//! - Timestamps emit as Unix-millis.

use async_trait::async_trait;
use reqwest::Client;

use crate::error::{Error, Result};
use crate::sink::{MetricPoint, MetricSink};

#[derive(Clone)]
pub struct VictoriaMetricsSink {
    base_url: String,
    http: Client,
}

impl VictoriaMetricsSink {
    /// Construct a sink against a VM HTTP base URL (e.g. `http://localhost:8428`).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http: Client::new(),
        }
    }

    fn ingest_url(&self) -> String {
        format!("{}/api/v1/import/prometheus", self.base_url)
    }

    /// Render a single point in Prom exposition format.
    /// Each call appends one full line including newline.
    fn render_point(&self, p: &MetricPoint, out: &mut String) {
        out.push_str(&sanitize_metric_name(&p.name));
        out.push('{');
        write_label(out, "source", &p.source, true);
        write_label(out, "source_id", &p.source_id, false);
        if let Some(obj) = p.labels.as_object() {
            for (k, v) in obj {
                let k = sanitize_label_name(k);
                let v = match v {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Null => continue,
                    other => other.to_string(),
                };
                write_label(out, &k, &v, false);
            }
        }
        out.push('}');
        out.push(' ');
        // VM accepts NaN/Inf; render finite/non-finite consistently.
        if p.value.is_finite() {
            out.push_str(&format!("{}", p.value));
        } else if p.value.is_nan() {
            out.push_str("NaN");
        } else if p.value.is_sign_positive() {
            out.push_str("+Inf");
        } else {
            out.push_str("-Inf");
        }
        out.push(' ');
        out.push_str(&p.ts.timestamp_millis().to_string());
        out.push('\n');
    }
}

#[async_trait]
impl MetricSink for VictoriaMetricsSink {
    async fn push(&self, point: MetricPoint) -> Result<()> {
        self.push_batch(vec![point]).await
    }

    async fn push_batch(&self, points: Vec<MetricPoint>) -> Result<()> {
        if points.is_empty() {
            return Ok(());
        }
        let mut body = String::with_capacity(points.len() * 128);
        for p in &points {
            self.render_point(p, &mut body);
        }
        let resp = self
            .http
            .post(self.ingest_url())
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(body)
            .send()
            .await
            .map_err(|e| Error::Other(format!("vm ingest send: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Other(format!(
                "vm ingest {} from {}: {}",
                status.as_u16(),
                self.ingest_url(),
                body.chars().take(200).collect::<String>(),
            )));
        }
        Ok(())
    }
}

/// Replace any non-`[a-zA-Z0-9_]` char with `_`; ensure the first char is not a digit.
fn sanitize_metric_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        let ok = c.is_ascii_alphanumeric() || c == '_';
        let ok = if i == 0 { ok && !c.is_ascii_digit() } else { ok };
        if ok {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "unnamed".into()
    } else {
        out
    }
}

fn sanitize_label_name(s: &str) -> String {
    sanitize_metric_name(s)
}

fn write_label(out: &mut String, key: &str, value: &str, first: bool) {
    if !first {
        out.push(',');
    }
    out.push_str(key);
    out.push_str("=\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn point() -> MetricPoint {
        MetricPoint {
            ts: chrono::Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            source: "pve".into(),
            source_id: "cluster-a".into(),
            name: "pve.node.cpu_pct".into(),
            value: 42.5,
            labels: serde_json::json!({"node": "pve-01", "vmid": 100}),
        }
    }

    #[test]
    fn renders_prom_text() {
        let sink = VictoriaMetricsSink::new("http://x");
        let mut buf = String::new();
        sink.render_point(&point(), &mut buf);
        assert!(buf.starts_with("pve_node_cpu_pct{"));
        assert!(buf.contains(r#"source="pve""#));
        assert!(buf.contains(r#"source_id="cluster-a""#));
        assert!(buf.contains(r#"node="pve-01""#));
        assert!(buf.contains(r#"vmid="100""#));
        assert!(buf.contains(" 42.5 "));
        assert!(buf.trim_end().ends_with("1700000000000"));
    }

    #[test]
    fn sanitises_metric_name() {
        assert_eq!(sanitize_metric_name("pve.node.cpu_pct"), "pve_node_cpu_pct");
        assert_eq!(sanitize_metric_name("agent.llm.input_tokens"), "agent_llm_input_tokens");
        assert_eq!(sanitize_metric_name("9foo"), "_foo");
    }

    #[test]
    fn handles_non_finite_values() {
        let sink = VictoriaMetricsSink::new("http://x");
        let mut p = point();
        p.value = f64::NAN;
        let mut buf = String::new();
        sink.render_point(&p, &mut buf);
        assert!(buf.contains(" NaN "));
        p.value = f64::INFINITY;
        let mut buf = String::new();
        sink.render_point(&p, &mut buf);
        assert!(buf.contains(" +Inf "));
    }
}
