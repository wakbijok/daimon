//! Bolt client over NornicDB.

use async_trait::async_trait;
use neo4rs::{ConfigBuilder, Graph};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::types::{BlastRadiusEntry, GraphPlan, NodeKind, TargetRef};

/// Graph-tier API. Implementations live in this crate (`NornicGraphClient`)
/// but the trait stays object-safe so consumers can swap to ArangoDB or
/// Neo4j Community later if the NornicDB bet doesn't pan out (see
/// MASTERPLAN §3.6 walkaway plan).
#[async_trait]
pub trait GraphClient: Send + Sync {
    /// Persist a plan + its steps + the depends-on graph + the target +
    /// capability references. Idempotent on (plan.id).
    async fn persist_plan(&self, plan: &GraphPlan) -> Result<()>;

    /// Blast-radius lookup for an approval-inbox summary. Returns the
    /// dependency set within `max_depth` hops of the target, ordered by
    /// depth ascending.
    async fn blast_radius(
        &self,
        tenant_id: Uuid,
        target_ref: &TargetRef,
        max_depth: u32,
    ) -> Result<Vec<BlastRadiusEntry>>;

    /// Upsert a target node so later blast-radius queries have something to
    /// traverse from. Idempotent on (tenant_id, target_ref).
    async fn upsert_target(
        &self,
        tenant_id: Uuid,
        target_ref: &TargetRef,
        labels: serde_json::Value,
    ) -> Result<()>;

    /// Declare a dependency edge between two targets.
    async fn declare_dependency(
        &self,
        tenant_id: Uuid,
        from: &TargetRef,
        to: &TargetRef,
    ) -> Result<()>;
}

#[derive(Clone)]
pub struct NornicGraphClient {
    graph: Graph,
}

impl NornicGraphClient {
    /// Connect to NornicDB over Bolt (default `bolt://localhost:7687`).
    /// `auth_user` / `auth_pass` may be empty strings for an unsecured dev
    /// instance — NornicDB ignores auth on the lite distribution.
    pub async fn connect(uri: &str, auth_user: &str, auth_pass: &str) -> Result<Self> {
        let cfg = ConfigBuilder::default()
            .uri(uri)
            .user(auth_user)
            .password(auth_pass)
            .build()
            .map_err(|e| Error::Other(format!("config: {e}")))?;
        let graph = Graph::connect(cfg)
            .await
            .map_err(|e| Error::Other(format!("connect: {e}")))?;
        Ok(Self { graph })
    }

    /// Borrow the underlying `neo4rs::Graph` — for schema bootstrap +
    /// ad-hoc Cypher used in tests.
    pub fn raw(&self) -> &Graph {
        &self.graph
    }
}

#[async_trait]
impl GraphClient for NornicGraphClient {
    async fn persist_plan(&self, plan: &GraphPlan) -> Result<()> {
        // MERGE the Plan node first.
        let q = neo4rs::query(
            "MERGE (p:Plan {id: $plan_id}) \
               ON CREATE SET p.tenant_id = $tenant_id, \
                             p.intent = $intent, \
                             p.created_at = $created_at",
        )
        .param("plan_id", plan.id.to_string())
        .param("tenant_id", plan.tenant_id.to_string())
        .param("intent", plan.intent.clone())
        .param("created_at", plan.created_at.to_rfc3339());
        self.graph.run(q).await?;

        // Then one round-trip per step. UNWIND-with-array params would be
        // more efficient but neo4rs 0.8's `BoltType: Into` doesn't accept
        // `serde_json::Value::Array` directly; per-step queries are
        // operationally fine — plans have a handful of steps, not
        // thousands.
        for s in &plan.steps {
            let q = neo4rs::query(
                "MATCH (p:Plan {id: $plan_id}) \
                 MERGE (s:PlanStep {id: $step_id}) \
                   ON CREATE SET s.capability_name = $name, \
                                 s.capability_version = $version, \
                                 s.target_ref = $target_ref \
                 MERGE (s)-[:STEP_OF]->(p) \
                 MERGE (c:Capability {name: $name, version: $version}) \
                 MERGE (s)-[:PROVIDES_CAPABILITY]->(c) \
                 MERGE (t:Target {ref: $target_ref}) \
                   ON CREATE SET t.tenant_id = $tenant_id \
                 MERGE (s)-[:DEPENDS_ON_TARGET]->(t)",
            )
            .param("plan_id", plan.id.to_string())
            .param("tenant_id", plan.tenant_id.to_string())
            .param("step_id", s.id.to_string())
            .param("name", s.capability_name.clone())
            .param("version", s.capability_version.clone())
            .param("target_ref", s.target_ref.as_str().to_string());
            self.graph.run(q).await?;
        }

        // Second pass: persist the depends_on edges between PlanSteps.
        for s in &plan.steps {
            for dep in &s.depends_on {
                let q = neo4rs::query(
                    "MATCH (a:PlanStep {id: $from}), (b:PlanStep {id: $to}) \
                     MERGE (a)-[:DEPENDS_ON]->(b)",
                )
                .param("from", s.id.to_string())
                .param("to", dep.to_string());
                self.graph.run(q).await?;
            }
        }
        Ok(())
    }

    async fn blast_radius(
        &self,
        tenant_id: Uuid,
        target_ref: &TargetRef,
        max_depth: u32,
    ) -> Result<Vec<BlastRadiusEntry>> {
        let max_depth = max_depth.clamp(1, 8);
        // Variable-length path traversal up to max_depth. Collect nodes
        // reachable from the target through any relationship; sort by hop
        // count ascending so the approval UI shows nearest-impact first.
        //
        // The query uses string interpolation for max_depth because some
        // Bolt servers don't accept depth as a parameter in path patterns.
        let cypher = format!(
            "MATCH (t:Target {{ref: $target_ref, tenant_id: $tenant_id}}) \
             MATCH path = (t)-[*1..{max_depth}]-(n) \
             WITH DISTINCT n, length(path) AS depth \
             RETURN labels(n) AS labels, \
                    coalesce(n.id, n.ref, n.name) AS id, \
                    coalesce(n.name, n.ref, n.id) AS label, \
                    min(depth) AS depth \
             ORDER BY depth ASC, label ASC \
             LIMIT 200"
        );
        let q = neo4rs::query(&cypher)
            .param("target_ref", target_ref.as_str().to_string())
            .param("tenant_id", tenant_id.to_string());

        let mut stream = self.graph.execute(q).await?;
        let mut out = Vec::new();
        while let Some(row) = stream.next().await? {
            let labels: Vec<String> =
                row.get("labels").map_err(|e| Error::Decode(e.to_string()))?;
            let id: String = row.get("id").map_err(|e| Error::Decode(e.to_string()))?;
            let label: String = row.get("label").map_err(|e| Error::Decode(e.to_string()))?;
            let depth: i64 = row.get("depth").map_err(|e| Error::Decode(e.to_string()))?;
            let kind = labels
                .first()
                .and_then(|l| node_kind_from_label(l))
                .unwrap_or(NodeKind::Target);
            out.push(BlastRadiusEntry {
                kind,
                id,
                label,
                depth: depth.max(0) as u32,
            });
        }
        Ok(out)
    }

    async fn upsert_target(
        &self,
        tenant_id: Uuid,
        target_ref: &TargetRef,
        labels: serde_json::Value,
    ) -> Result<()> {
        let q = neo4rs::query(
            "MERGE (t:Target {ref: $ref}) \
               ON CREATE SET t.tenant_id = $tenant_id, t.labels = $labels \
               ON MATCH SET t.labels = $labels",
        )
        .param("ref", target_ref.as_str().to_string())
        .param("tenant_id", tenant_id.to_string())
        .param("labels", labels.to_string());
        self.graph.run(q).await?;
        Ok(())
    }

    async fn declare_dependency(
        &self,
        tenant_id: Uuid,
        from: &TargetRef,
        to: &TargetRef,
    ) -> Result<()> {
        let q = neo4rs::query(
            "MATCH (a:Target {ref: $from, tenant_id: $tenant_id}), \
                   (b:Target {ref: $to, tenant_id: $tenant_id}) \
             MERGE (a)-[:DEPENDS_ON_TARGET]->(b)",
        )
        .param("from", from.as_str().to_string())
        .param("to", to.as_str().to_string())
        .param("tenant_id", tenant_id.to_string());
        self.graph.run(q).await?;
        Ok(())
    }
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
