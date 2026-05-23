//! Phase 7 — `/admin/observer` server-fns.
//!
//! Read-only views of the observer.metrics + observer.anomalies tables.
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
             WHERE tenant_id = $1
             ORDER BY detected_at DESC
             LIMIT $2",
            &[&state.tenant_id, &(limit as i64)],
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
    let client = state
        .db
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool: {e}")))?;
    let rows = client
        .query(
            "SELECT source, source_id, name,
                    (ARRAY_AGG(value ORDER BY ts DESC))[1] AS last_value,
                    MAX(ts) AS last_ts,
                    COUNT(*) AS sample_count
             FROM observer.metrics
             WHERE tenant_id = $1
             GROUP BY source, source_id, name
             ORDER BY last_ts DESC
             LIMIT $2",
            &[&state.tenant_id, &(limit as i64)],
        )
        .await
        .map_err(|e| ServerFnError::new(format!("query: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let last_ts: chrono::DateTime<chrono::Utc> = r.get(4);
            MetricSummaryRow {
                source: r.get(0),
                source_id: r.get(1),
                name: r.get(2),
                last_value: r.get(3),
                last_ts: last_ts.to_rfc3339(),
                sample_count: r.get(5),
            }
        })
        .collect())
}
