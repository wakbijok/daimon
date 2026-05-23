//! Metric sink — appends rows to `observer.metrics`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use daimon_db::Pool;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
    async fn push(&self, tenant_id: Uuid, point: MetricPoint) -> Result<()>;

    async fn push_batch(&self, tenant_id: Uuid, points: Vec<MetricPoint>) -> Result<()> {
        for p in points {
            self.push(tenant_id, p).await?;
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct PostgresMetricSink {
    pool: Pool,
}

impl PostgresMetricSink {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MetricSink for PostgresMetricSink {
    async fn push(&self, tenant_id: Uuid, point: MetricPoint) -> Result<()> {
        let client = self.pool.get().await?;
        let labels = if point.labels.is_null() {
            serde_json::Value::Object(serde_json::Map::new())
        } else {
            point.labels
        };
        client
            .execute(
                "INSERT INTO observer.metrics
                    (ts, tenant_id, source, source_id, name, value, labels)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[
                    &point.ts,
                    &tenant_id,
                    &point.source,
                    &point.source_id,
                    &point.name,
                    &point.value,
                    &labels,
                ],
            )
            .await?;
        Ok(())
    }

    async fn push_batch(&self, tenant_id: Uuid, points: Vec<MetricPoint>) -> Result<()> {
        if points.is_empty() {
            return Ok(());
        }
        let mut client = self.pool.get().await?;
        let txn = client.transaction().await?;
        let stmt = txn
            .prepare(
                "INSERT INTO observer.metrics
                    (ts, tenant_id, source, source_id, name, value, labels)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .await?;
        for p in points {
            let labels = if p.labels.is_null() {
                serde_json::Value::Object(serde_json::Map::new())
            } else {
                p.labels
            };
            txn.execute(
                &stmt,
                &[
                    &p.ts,
                    &tenant_id,
                    &p.source,
                    &p.source_id,
                    &p.name,
                    &p.value,
                    &labels,
                ],
            )
            .await?;
        }
        txn.commit().await?;
        Ok(())
    }
}
