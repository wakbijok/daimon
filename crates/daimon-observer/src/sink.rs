//! Metric sink — pushes metric points to a time-series backend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricPoint {
    pub ts: DateTime<Utc>,
    pub source: String,
    pub source_id: String,
    pub name: String,
    pub value: f64,
    #[serde(default)]
    pub labels: serde_json::Value,
}

#[async_trait]
pub trait MetricSink: Send + Sync {
    async fn push(&self, point: MetricPoint) -> Result<()>;

    async fn push_batch(&self, points: Vec<MetricPoint>) -> Result<()> {
        for p in points {
            self.push(p).await?;
        }
        Ok(())
    }
}
