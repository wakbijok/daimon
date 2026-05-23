//! Minimal Prometheus client. Supports instant queries via /api/v1/query.

use reqwest::Client;
use serde::Deserialize;
use serde_json::Value as Json;

use crate::error::{Error, Result};

#[derive(Clone)]
pub struct PrometheusClient {
    http: Client,
    base_url: String,
}

impl PrometheusClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        })
    }

    /// Run an instant PromQL query. Returns the resolved vector of
    /// `(labels, value)` pairs.
    pub async fn instant(&self, query: &str) -> Result<Vec<InstantSample>> {
        let url = format!("{}/api/v1/query", self.base_url);
        let resp = self.http.get(&url).query(&[("query", query)]).send().await?;
        if !resp.status().is_success() {
            return Err(Error::Api(format!(
                "instant {query}: status {}",
                resp.status()
            )));
        }
        let body: PromResponse = resp.json().await.map_err(Error::Http)?;
        if body.status != "success" {
            return Err(Error::Api(format!("instant {query}: {:?}", body.error)));
        }
        let data = body.data.ok_or_else(|| Error::Api("no data".into()))?;
        if data.result_type != "vector" {
            return Err(Error::Api(format!(
                "instant {query}: unexpected result type {}",
                data.result_type
            )));
        }
        let mut out = Vec::new();
        for r in data.result.as_array().cloned().unwrap_or_default() {
            let metric = r
                .get("metric")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let value_arr = r
                .get("value")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if value_arr.len() < 2 {
                continue;
            }
            let ts_f = value_arr
                .first()
                .and_then(|v| v.as_f64())
                .unwrap_or_default();
            let val_str = value_arr.get(1).and_then(|v| v.as_str()).unwrap_or("0");
            let val: f64 = val_str.parse().unwrap_or(0.0);
            out.push(InstantSample {
                metric,
                timestamp_secs: ts_f,
                value: val,
            });
        }
        Ok(out)
    }
}

#[derive(Debug, Clone)]
pub struct InstantSample {
    pub metric: Json,
    pub timestamp_secs: f64,
    pub value: f64,
}

#[derive(Debug, Deserialize)]
struct PromResponse {
    status: String,
    #[serde(default)]
    data: Option<PromData>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PromData {
    #[serde(rename = "resultType")]
    result_type: String,
    result: Json,
}
