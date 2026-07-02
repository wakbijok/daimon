//! daimon-memory — the memory tier seam for daimon agents.
//!
//! P3 LOCKED decision: the long-term memory tier is a **dmem SIDECAR**, not an
//! embedded store. A musl-static daimon binary cannot link dm-lite's native
//! zvec `.so`, so daimon talks to a running `dmem serve` over HTTP behind the
//! [`MemoryService`] trait. The prior Qdrant [`vector`]-store impl and the
//! `daimon-rag` embedding pipeline are retired.
//!
//! - [`service`] — the trait + transport-agnostic DTOs (all `serde`; the
//!   hydrate/wasm side renders these without pulling `reqwest`).
//! - [`dmem_http`] — the concrete sidecar client [`DmemHttpMemory`] plus the
//!   no-op [`NullMemory`] boot fallback (ssr-only; the sole `reqwest` user).

pub mod dmem_http;
pub mod error;
pub mod service;

pub use dmem_http::{DmemHttpMemory, NullMemory};
pub use error::{Error, Result};
pub use service::{
    IngestDoc, IngestStats, MemoryHealth, MemoryService, PreTurnContext, RecallBudget, RecordKind,
    RetrieveQuery, RetrievedChunk, ScoredRecord, TypedBody, TypedRecord,
};
