//! Bootstrap Cypher — node uniqueness constraints + relationship indexes.
//!
//! NornicDB inherits Neo4j Cypher semantics for `CREATE CONSTRAINT` and
//! `CREATE INDEX`. The constraint statements are idempotent
//! (`IF NOT EXISTS`) so this can run on every boot.

use neo4rs::Graph;

use crate::error::{Error, Result};

pub const SCHEMA_CYPHER: &[&str] = &[
    // Node uniqueness constraints — one row per logical identity.
    "CREATE CONSTRAINT tenant_id_unique IF NOT EXISTS FOR (t:Tenant) REQUIRE t.id IS UNIQUE",
    "CREATE CONSTRAINT agent_id_unique IF NOT EXISTS FOR (a:Agent) REQUIRE a.id IS UNIQUE",
    "CREATE CONSTRAINT capability_namever_unique IF NOT EXISTS FOR (c:Capability) REQUIRE (c.name, c.version) IS UNIQUE",
    "CREATE CONSTRAINT target_ref_unique IF NOT EXISTS FOR (t:Target) REQUIRE t.ref IS UNIQUE",
    "CREATE CONSTRAINT credential_ref_unique IF NOT EXISTS FOR (c:Credential) REQUIRE c.ref IS UNIQUE",
    "CREATE CONSTRAINT plan_id_unique IF NOT EXISTS FOR (p:Plan) REQUIRE p.id IS UNIQUE",
    "CREATE CONSTRAINT planstep_id_unique IF NOT EXISTS FOR (s:PlanStep) REQUIRE s.id IS UNIQUE",
    "CREATE CONSTRAINT auditevent_id_unique IF NOT EXISTS FOR (e:AuditEvent) REQUIRE e.id IS UNIQUE",
    "CREATE CONSTRAINT user_id_unique IF NOT EXISTS FOR (u:User) REQUIRE u.id IS UNIQUE",
    // Lookup indexes — most blast-radius queries enter from a Target or Credential.
    "CREATE INDEX target_tenant_idx IF NOT EXISTS FOR (t:Target) ON (t.tenant_id)",
    "CREATE INDEX credential_tenant_idx IF NOT EXISTS FOR (c:Credential) ON (c.tenant_id)",
    "CREATE INDEX plan_tenant_idx IF NOT EXISTS FOR (p:Plan) ON (p.tenant_id, p.created_at)",
];

/// Apply the schema bootstrap against an open Bolt connection.
pub async fn ensure_schema(graph: &Graph) -> Result<()> {
    for stmt in SCHEMA_CYPHER {
        graph
            .run(neo4rs::query(stmt))
            .await
            .map_err(|e| Error::Schema(format!("{stmt} failed: {e}")))?;
    }
    Ok(())
}
