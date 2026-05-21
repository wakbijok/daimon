//! Append-only audit event log (D23).
//!
//! Every broker `execute` writes one or more audit events. Future Guard
//! decisions (Phase 5) and Orchestrator plan-step events (Phase 6) also
//! flow through here. Schema is append-only — DB-level triggers block
//! UPDATE and DELETE.
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
//! See `docs/specs/2026-05-20-multi-agent-architecture-design.md` D23.

pub mod event;
pub mod sink;
pub mod sqlite_sink;

pub use event::{ActionKind, AuditEvent, AuditFilter, AuditResult, NewAuditEvent};
pub use sink::{AuditError, AuditSink};
pub use sqlite_sink::SqliteAuditSink;
