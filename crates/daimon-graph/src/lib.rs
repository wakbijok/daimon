//! Graph tier (#5 of the 5-DB storage architecture).
//!
//! Backed by **NornicDB** (Phase 8 lock per MASTERPLAN §3.6 amendment
//! 2026-05-23 late). Speaks Bolt/Cypher via `neo4rs`, which works against
//! NornicDB's Neo4j-wire-compatible endpoint at :7687.
//!
//! Two load-bearing primitives Phase 8 needs:
//!
//! 1. **Blast radius** — given a `target_ref`, return the set of dependent
//!    nodes (workloads on the same VLAN, tenants sharing the credential,
//!    plans referencing the target) so Guard can show an approval-inbox
//!    summary before letting the operator green-light a write.
//!
//! 2. **Plan persistence** — Orchestrator persists every emitted `Plan` to
//!    the graph alongside Postgres. Postgres is the canonical store for
//!    SQL queries (plan_inspector UI); the graph is for cross-reference
//!    queries ("show me every plan that ever touched target X").
//!
//! Schema (see MASTERPLAN §3.6 schema-shape block):
//!
//! - Nodes: `Tenant`, `Agent`, `Capability`, `Target`, `Credential`, `Plan`,
//!   `PlanStep`, `AuditEvent`, `User`
//! - Edges: `OWNS`, `EXECUTES_AS`, `PROVIDES_CAPABILITY`,
//!   `DEPENDS_ON_TARGET`, `REQUIRES_CREDENTIAL`, `EMITTED_BY`, `STEP_OF`,
//!   `BLAST_RADIUS`
//!
//! Multi-tenant isolation: per-tenant Cypher `USE` database for queries.
//! The connection pool caches per-tenant sessions.

pub mod client;
pub mod error;
pub mod schema;
pub mod types;

pub use client::{GraphClient, NornicGraphClient};
pub use error::{Error, Result};
pub use schema::{ensure_schema, SCHEMA_CYPHER};
pub use types::{
    BlastRadiusEntry, GraphPlan, GraphPlanStep, NodeKind, RelKind, TargetRef,
};
