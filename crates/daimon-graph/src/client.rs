//! HTTP-Cypher client for NornicDB.
//!
//! Uses the Neo4j-compatible transactional HTTP API at
//! `POST /db/<database>/tx/commit`. NornicDB's Bolt server advertises
//! Bolt 4.4 only; modern Rust Bolt drivers (neo4rs 0.8+) speak Bolt 5
//! and reject 4.4 during negotiation. HTTP avoids the wire mismatch and
//! works against the same Cypher engine. Swap back to Bolt when
//! NornicDB ships Bolt 5 or a Rust 4.4-capable driver lands.
//!
//! The trait `GraphClient` stays the same so callers don't care which
//! transport we use.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::types::{BlastRadiusEntry, GraphPlan, NodeKind, TargetRef};

#[async_trait]
pub trait GraphClient: Send + Sync {
    /// Persist a plan + its steps + the depends-on graph. Idempotent on plan.id.
    async fn persist_plan(&self, plan: &GraphPlan) -> Result<()>;

    /// Blast-radius lookup. Returns reachable nodes up to `max_depth` hops,
    /// ordered by depth ascending.
    async fn blast_radius(
        &self,
        target_ref: &TargetRef,
        max_depth: u32,
    ) -> Result<Vec<BlastRadiusEntry>>;

    /// Upsert a target node. Idempotent on `target_ref`.
    async fn upsert_target(
        &self,
        target_ref: &TargetRef,
        labels: serde_json::Value,
    ) -> Result<()>;

    /// Declare a dependency edge between two targets.
    async fn declare_dependency(&self, from: &TargetRef, to: &TargetRef) -> Result<()>;
}

#[derive(Clone)]
pub struct NornicGraphClient {
    base_url: String,
    database: String,
    http: Client,
}

impl NornicGraphClient {
    /// Construct a client from a NornicDB HTTP URL (e.g.
    /// `http://localhost:7474`) and a target database name.
    pub fn new(base_url: impl Into<String>, database: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            database: database.into(),
            http: Client::new(),
        }
    }

    /// Convenience: accept a Bolt URL and translate to the HTTP endpoint.
    /// `bolt://localhost:7687` → `http://localhost:7474`.
    pub async fn connect(uri: &str, _auth_user: &str, _auth_pass: &str) -> Result<Self> {
        let http_url = bolt_to_http(uri)?;
        Ok(Self::new(http_url, "nornic"))
    }

    fn tx_url(&self) -> String {
        format!("{}/db/{}/tx/commit", self.base_url, self.database)
    }

    /// Run one or more Cypher statements in a single committed transaction.
    /// Each `(query, params)` becomes one `{statement, parameters}` entry.
    /// Returns the parsed result columns + rows for each statement, in order.
    pub async fn run(&self, statements: Vec<(String, Value)>) -> Result<Vec<CypherResult>> {
        if statements.is_empty() {
            return Ok(Vec::new());
        }
        let body = json!({
            "statements": statements
                .iter()
                .map(|(stmt, params)| json!({
                    "statement": stmt,
                    "parameters": params,
                }))
                .collect::<Vec<_>>()
        });
        let raw = self
            .http
            .post(self.tx_url())
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let resp: TxResponse = serde_json::from_str(&raw).map_err(|e| {
            Error::Decode(format!(
                "tx response parse: {e}; body: {}",
                raw.chars().take(500).collect::<String>()
            ))
        })?;
        if let Some(err) = resp.errors.into_iter().next() {
            return Err(Error::Cypher {
                code: err.code,
                message: err.message,
            });
        }
        Ok(resp.results)
    }

    /// Single-statement convenience.
    async fn run_one(&self, statement: &str, params: Value) -> Result<CypherResult> {
        let mut rs = self.run(vec![(statement.into(), params)]).await?;
        rs.pop().ok_or_else(|| Error::Decode("no result returned".into()))
    }
}

#[async_trait]
impl GraphClient for NornicGraphClient {
    async fn persist_plan(&self, plan: &GraphPlan) -> Result<()> {
        // NornicDB v1.1.1 parser bug (observed 2026-05-23): a chained
        // statement containing MATCH → MERGE → ON CREATE SET → MERGE
        // (relationship) breaks parameter substitution inside the SET — the
        // assigned property becomes the literal `true` instead of the
        // parameter value. Workaround: emit one MERGE/MATCH primary clause
        // per statement. All statements ship in a single tx/commit so this
        // stays atomic + idempotent.
        let mut batch: Vec<(String, Value)> = Vec::new();

        // 1) Plan node (single MERGE, no chain).
        batch.push((
            "MERGE (p:Plan {id: $pid}) \
               ON CREATE SET p.intent = $intent, \
                             p.created_at = $created_at"
                .into(),
            json!({
                "pid": plan.id.to_string(),
                "intent": plan.intent,
                "created_at": plan.created_at.to_rfc3339(),
            }),
        ));

        for s in &plan.steps {
            // 2) PlanStep node (single MERGE, no chain).
            batch.push((
                "MERGE (s:PlanStep {id: $sid}) \
                   ON CREATE SET s.capability_name = $cname, \
                                 s.capability_version = $cver, \
                                 s.target_ref = $tref"
                    .into(),
                json!({
                    "sid": s.id.to_string(),
                    "cname": s.capability_name,
                    "cver": s.capability_version,
                    "tref": s.target_ref.as_str(),
                }),
            ));
            // 3) STEP_OF rel (pure MATCH+MERGE rel, no SET).
            batch.push((
                "MATCH (p:Plan {id: $pid}), (s:PlanStep {id: $sid}) \
                 MERGE (s)-[:STEP_OF]->(p)"
                    .into(),
                json!({"pid": plan.id.to_string(), "sid": s.id.to_string()}),
            ));
            // 4) Capability node.
            batch.push((
                "MERGE (c:Capability {name: $cname, version: $cver})".into(),
                json!({"cname": s.capability_name, "cver": s.capability_version}),
            ));
            // 5) PROVIDES_CAPABILITY rel.
            batch.push((
                "MATCH (s:PlanStep {id: $sid}), (c:Capability {name: $cname, version: $cver}) \
                 MERGE (s)-[:PROVIDES_CAPABILITY]->(c)"
                    .into(),
                json!({
                    "sid": s.id.to_string(),
                    "cname": s.capability_name,
                    "cver": s.capability_version,
                }),
            ));
            // 6) Target node (single MERGE).
            batch.push((
                "MERGE (t:Target {ref: $tref})".into(),
                json!({"tref": s.target_ref.as_str()}),
            ));
            // 7) DEPENDS_ON_TARGET rel.
            batch.push((
                "MATCH (s:PlanStep {id: $sid}), (t:Target {ref: $tref}) \
                 MERGE (s)-[:DEPENDS_ON_TARGET]->(t)"
                    .into(),
                json!({"sid": s.id.to_string(), "tref": s.target_ref.as_str()}),
            ));
        }

        // 8) Depends-on edges between PlanSteps.
        for s in &plan.steps {
            for dep in &s.depends_on {
                batch.push((
                    "MATCH (a:PlanStep {id: $from_sid}), (b:PlanStep {id: $to_sid}) \
                     MERGE (a)-[:DEPENDS_ON]->(b)"
                        .into(),
                    json!({"from_sid": s.id.to_string(), "to_sid": dep.to_string()}),
                ));
            }
        }

        self.run(batch).await?;
        Ok(())
    }

    async fn blast_radius(
        &self,
        target_ref: &TargetRef,
        max_depth: u32,
    ) -> Result<Vec<BlastRadiusEntry>> {
        let max_depth = max_depth.clamp(1, 8);
        // NornicDB v1.1.1 quirk: `WITH DISTINCT n, length(path) AS d ...
        // RETURN min(d)` collapses to a single row even when n varies.
        // Workaround is to compute min depth in the RETURN itself with an
        // implicit grouping on the non-aggregate columns — that path
        // returns the expected per-node minimum depth.
        let cypher = format!(
            "MATCH (t:Target {{ref: $tref}}) \
             MATCH path = (t)-[*1..{max_depth}]-(n) \
             RETURN labels(n) AS labels, \
                    coalesce(n.id, n.ref, n.name) AS nid, \
                    coalesce(n.name, n.ref, n.id) AS nlabel, \
                    min(length(path)) AS depth \
             ORDER BY depth ASC, nlabel ASC \
             LIMIT 200"
        );
        let res = self
            .run_one(&cypher, json!({"tref": target_ref.as_str()}))
            .await?;
        let mut out = Vec::with_capacity(res.data.len());
        for row in res.data {
            let cells = &row.row;
            if cells.len() < 4 {
                continue;
            }
            let labels: Vec<String> = cells[0]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let id = value_to_string(&cells[1]);
            let label = value_to_string(&cells[2]);
            let depth = cells[3].as_i64().unwrap_or(0).max(0) as u32;
            let kind = labels
                .first()
                .and_then(|l| node_kind_from_label(l))
                .unwrap_or(NodeKind::Target);
            out.push(BlastRadiusEntry { kind, id, label, depth });
        }
        Ok(out)
    }

    async fn upsert_target(&self, target_ref: &TargetRef, labels: serde_json::Value) -> Result<()> {
        self.run_one(
            "MERGE (t:Target {ref: $tref}) \
               ON CREATE SET t.labels = $lbl \
               ON MATCH SET t.labels = $lbl",
            json!({
                "tref": target_ref.as_str(),
                "lbl": labels.to_string(),
            }),
        )
        .await?;
        Ok(())
    }

    async fn declare_dependency(&self, from: &TargetRef, to: &TargetRef) -> Result<()> {
        self.run_one(
            "MATCH (a:Target {ref: $from_tref}), \
                   (b:Target {ref: $to_tref}) \
             MERGE (a)-[:DEPENDS_ON_TARGET]->(b)",
            json!({
                "from_tref": from.as_str(),
                "to_tref": to.as_str(),
            }),
        )
        .await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct TxResponse {
    #[serde(default)]
    pub results: Vec<CypherResult>,
    #[serde(default)]
    pub errors: Vec<CypherError>,
}

#[derive(Debug, Deserialize)]
pub struct CypherResult {
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub data: Vec<CypherRow>,
}

#[derive(Debug, Deserialize)]
pub struct CypherRow {
    #[serde(default)]
    pub row: Vec<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CypherError {
    pub code: String,
    pub message: String,
}

fn node_kind_from_label(s: &str) -> Option<NodeKind> {
    Some(match s {
        "Tenant" => NodeKind::Tenant,
        "Agent" => NodeKind::Agent,
        "Capability" => NodeKind::Capability,
        "Target" => NodeKind::Target,
        "Credential" => NodeKind::Credential,
        "Plan" => NodeKind::Plan,
        "PlanStep" => NodeKind::PlanStep,
        "AuditEvent" => NodeKind::AuditEvent,
        "User" => NodeKind::User,
        _ => return None,
    })
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn bolt_to_http(uri: &str) -> Result<String> {
    let url = url::Url::parse(uri).map_err(|e| Error::Other(format!("uri parse: {e}")))?;
    let host = url.host_str().ok_or_else(|| Error::Other("uri missing host".into()))?;
    // Convention: Bolt port = HTTP port + 213 (7474 → 7687). Allow override
    // via env so prod can deviate. For now hardcode the dev mapping.
    let http_port = url
        .port()
        .map(|p| if p == 7687 { 7474 } else { p })
        .unwrap_or(7474);
    Ok(format!("http://{host}:{http_port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bolt_to_http_default_ports() {
        assert_eq!(
            bolt_to_http("bolt://localhost:7687").unwrap(),
            "http://localhost:7474"
        );
    }

    #[test]
    fn bolt_to_http_passes_through_non_default() {
        assert_eq!(
            bolt_to_http("bolt://graph-host.internal:9999").unwrap(),
            "http://graph-host.internal:9999"
        );
    }
}
