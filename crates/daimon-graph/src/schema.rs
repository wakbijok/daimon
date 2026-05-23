//! Bootstrap Cypher — node uniqueness constraints + relationship indexes.
//!
//! Idempotent (`IF NOT EXISTS`) so this can run on every boot. Runs over
//! the HTTP transactional API via `NornicGraphClient`.

use crate::client::NornicGraphClient;
use crate::error::Result;

pub const SCHEMA_CYPHER: &[&str] = &[
    // Node uniqueness constraints — single-property only. NornicDB's
    // constraint engine doesn't accept composite-key REQUIRE clauses
    // (e.g. `(c.name, c.version) IS UNIQUE`) — application-level MERGE
    // semantics enforce the (name, version) uniqueness for Capability
    // instead.
    "CREATE CONSTRAINT tenant_id_unique IF NOT EXISTS FOR (t:Tenant) REQUIRE t.id IS UNIQUE",
    "CREATE CONSTRAINT agent_id_unique IF NOT EXISTS FOR (a:Agent) REQUIRE a.id IS UNIQUE",
    "CREATE CONSTRAINT target_ref_unique IF NOT EXISTS FOR (t:Target) REQUIRE t.ref IS UNIQUE",
    "CREATE CONSTRAINT credential_ref_unique IF NOT EXISTS FOR (c:Credential) REQUIRE c.ref IS UNIQUE",
    "CREATE CONSTRAINT plan_id_unique IF NOT EXISTS FOR (p:Plan) REQUIRE p.id IS UNIQUE",
    "CREATE CONSTRAINT planstep_id_unique IF NOT EXISTS FOR (s:PlanStep) REQUIRE s.id IS UNIQUE",
    "CREATE CONSTRAINT auditevent_id_unique IF NOT EXISTS FOR (e:AuditEvent) REQUIRE e.id IS UNIQUE",
    "CREATE CONSTRAINT user_id_unique IF NOT EXISTS FOR (u:User) REQUIRE u.id IS UNIQUE",
    // Lookup indexes — most blast-radius queries enter from a Target or Credential.
    "CREATE INDEX target_tenant_idx IF NOT EXISTS FOR (t:Target) ON (t.tenant_id)",
    "CREATE INDEX credential_tenant_idx IF NOT EXISTS FOR (c:Credential) ON (c.tenant_id)",
    "CREATE INDEX plan_tenant_idx IF NOT EXISTS FOR (p:Plan) ON (p.tenant_id)",
    "CREATE INDEX capability_name_idx IF NOT EXISTS FOR (c:Capability) ON (c.name)",
];

/// Apply the schema bootstrap. NornicDB's executor runs each statement
/// independently; we send them one at a time so a single failure doesn't
/// roll back the rest.
pub async fn ensure_schema(client: &NornicGraphClient) -> Result<()> {
    for stmt in SCHEMA_CYPHER {
        client.run(vec![(stmt.to_string(), serde_json::json!({}))]).await?;
    }
    Ok(())
}
