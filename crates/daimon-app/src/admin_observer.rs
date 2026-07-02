//! Phase 7/8 — `/admin/observer` server-fns.
//!
//! Anomaly rows still live in Postgres (`observer.anomalies`). Metric
//! streams moved to VictoriaMetrics in Phase 8 (V015 dropped the
//! Postgres `observer.metrics` table); `metric_summary` now queries VM's
//! HTTP API instead.
//! Both gated by `require_admin()`.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyRow {
    pub id: String,
    pub detected_at: String,
    pub source: String,
    pub source_id: String,
    pub severity: String,
    pub title: String,
    pub description: Option<String>,
    pub metric_name: Option<String>,
    pub metric_value: Option<f64>,
    pub threshold: Option<f64>,
    pub resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSummaryRow {
    pub source: String,
    pub source_id: String,
    pub name: String,
    pub last_value: f64,
    pub last_ts: String,
    pub sample_count: i64,
}

#[server]
pub async fn list_anomalies(limit: u32) -> Result<Vec<AnomalyRow>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let _claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let client = state
        .db
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool: {e}")))?;
    let rows = client
        .query(
            "SELECT id, detected_at, source, source_id, severity, title,
                    description, metric_name, metric_value, threshold, resolved_at
             FROM observer.anomalies
             ORDER BY detected_at DESC
             LIMIT $1",
            &[&(limit as i64)],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("query: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let id: uuid::Uuid = r.get(0);
            let detected_at: chrono::DateTime<chrono::Utc> = r.get(1);
            let resolved_at: Option<chrono::DateTime<chrono::Utc>> = r.get(10);
            AnomalyRow {
                id: id.to_string(),
                detected_at: detected_at.to_rfc3339(),
                source: r.get(2),
                source_id: r.get(3),
                severity: r.get(4),
                title: r.get(5),
                description: r.get(6),
                metric_name: r.get(7),
                metric_value: r.get(8),
                threshold: r.get(9),
                resolved: resolved_at.is_some(),
            }
        })
        .collect())
}

#[server]
pub async fn metric_summary(limit: u32) -> Result<Vec<MetricSummaryRow>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let _claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let _ = state;

    // Phase 8: metric streams live in VictoriaMetrics now. Query VM's
    // /api/v1/series for active series (last 24h window), then for each
    // unique (source, source_id, __name__) tuple shoot a follow-up instant
    // query for the last value.
    //
    // Single-org: metrics are no longer tenant-tagged, so the catalogue
    // matcher selects every source-tagged series.
    //
    // VM URL is the same env daimon-app's main.rs reads at boot for the
    // VictoriaMetricsSink; default to localhost:8428 when unset.
    let vm_url = std::env::var("DAIMON_VM_URL")
        .unwrap_or_else(|_| "http://localhost:8428".to_string());
    let vm_url = vm_url.trim_end_matches('/').to_string();
    let http = reqwest::Client::new();

    // Series catalogue. Match anything carrying a `source` label.
    let matcher = "{source!=\"\"}".to_string();
    let series_resp = http
        .get(format!("{vm_url}/api/v1/series"))
        .query(&[("match[]", matcher.as_str())])
        .send()
        .await
        .map_err(|e| ServerFnError::new(format!("vm series: {e}")))?
        .error_for_status()
        .map_err(|e| ServerFnError::new(format!("vm series status: {e}")))?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| ServerFnError::new(format!("vm series json: {e}")))?;

    // Build (name, source, source_id) keys from the series list. Dedupe
    // because /api/v1/series returns one entry per unique label set —
    // we only care about the (name, source, source_id) tuple for the
    // summary view.
    let empty: Vec<serde_json::Value> = Vec::new();
    let series = series_resp
        .get("data")
        .and_then(|d| d.as_array())
        .unwrap_or(&empty);

    let mut seen: std::collections::HashSet<(String, String, String)> =
        std::collections::HashSet::new();
    let mut keys: Vec<(String, String, String)> = Vec::new();
    for s in series {
        let name = s.get("__name__").and_then(|v| v.as_str()).unwrap_or("");
        let source = s.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let source_id = s.get("source_id").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let key = (name.to_string(), source.to_string(), source_id.to_string());
        if seen.insert(key.clone()) {
            keys.push(key);
        }
        if keys.len() >= limit as usize {
            break;
        }
    }

    // For each unique series, query the latest value with a 1h lookback.
    // Doing this in one PromQL pass (`last_over_time(...)`) would be
    // cheaper but the per-series shape needs the labels back too, which
    // requires multiple round-trips anyway. Phase 8.1: switch to a
    // single batched query if cardinality grows.
    let mut out = Vec::with_capacity(keys.len());
    for (name, source, source_id) in keys {
        let filter = format!(
            "{name}{{source=\"{source}\",source_id=\"{sid}\"}}[1h]",
            name = name,
            source = source,
            sid = source_id,
        );
        let resp = http
            .get(format!("{vm_url}/api/v1/query"))
            .query(&[("query", filter.as_str())])
            .send()
            .await
            .map_err(|e| ServerFnError::new(format!("vm query: {e}")))?;
        if !resp.status().is_success() {
            continue;
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ServerFnError::new(format!("vm query json: {e}")))?;
        let result = body
            .get("data")
            .and_then(|d| d.get("result"))
            .and_then(|r| r.as_array());
        if let Some(arr) = result {
            if let Some(first) = arr.first() {
                if let Some(values) = first.get("values").and_then(|v| v.as_array()) {
                    if let Some(last) = values.last() {
                        // VM range result: [ [ts_unix, "value_string"], ... ]
                        let ts_unix = last.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let val_s = last.get(1).and_then(|v| v.as_str()).unwrap_or("0");
                        let last_value = val_s.parse::<f64>().unwrap_or(0.0);
                        let last_ts = chrono::DateTime::<chrono::Utc>::from_timestamp(
                            ts_unix as i64,
                            ((ts_unix.fract()) * 1e9) as u32,
                        )
                        .map(|d| d.to_rfc3339())
                        .unwrap_or_default();
                        out.push(MetricSummaryRow {
                            source: source.clone(),
                            source_id: source_id.clone(),
                            name: name.clone(),
                            last_value,
                            last_ts,
                            sample_count: values.len() as i64,
                        });
                    }
                }
            }
        }
    }
    Ok(out)
}
