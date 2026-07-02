//! Append-only audit event log (D23).
//!
//! Every broker `execute` writes one or more audit events. Future Guard
//! decisions (Phase 5) and Orchestrator plan-step events (Phase 6) also
//! flow through here. Schema is append-only — DB-level triggers block
//! UPDATE and DELETE.
//!
//! Phase 2c D3b: storage moved from SQLite to PostgreSQL. A single-org hash
//! chain (V008 BEFORE INSERT trigger) provides tamper evidence; the
//! `daimon-anchor` operator binary snapshots + verifies. Event IDs are now
//! `Uuid` rather than `i64`.
//!
//! Workers do not depend on this crate; the broker writes events on their
//! behalf so a compromised worker cannot poison the log.
//!
//! Two readers:
//! - `daimon-broker` — writes via `AuditSink::append` and may query for
//!   correlation
//! - `daimon-app` — reads via `query` + `count` for the `/admin/audit` UI,
//!   gated by `require_admin()` (D24)
//!
//! See `daimon-docs/specs/2026-05-20-multi-agent-architecture-design.md` D23.

pub mod event;
pub mod postgres_sink;
pub mod sink;

pub use event::{ActionKind, AuditEvent, AuditFilter, AuditResult, NewAuditEvent};
pub use postgres_sink::PostgresAuditSink;
pub use sink::{AuditError, AuditSink};
