//! The `MemoryService` seam — daimon's abstract memory tier.
//!
//! P3 LOCKED decision: the memory tier is a **dmem SIDECAR**, not an embedded
//! store. A musl-static daimon binary cannot link dm-lite's native zvec `.so`,
//! so daimon talks to a running `dmem serve` over HTTP (see [`crate::dmem_http`]).
//!
//! This module defines only the trait + the transport-agnostic DTOs. Everything
//! here derives `Serialize`/`Deserialize` so the hydrate (wasm) side can render
//! the same types the ssr side produces — the concrete client
//! ([`crate::dmem_http::DmemHttpMemory`]) and its `reqwest` dependency stay
//! ssr-only and never enter the wasm bundle.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// A document to ingest into long-term memory. Free text keyed by a stable
/// source id + a coarse kind label (used only for provenance in the payload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestDoc {
    pub source_id: String,
    pub source_kind: String,
    pub content: String,
}

/// Result of an [`MemoryService::ingest`] call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestStats {
    pub source_id: String,
    /// Number of records written for this document. Under the sidecar each
    /// `remember` is one record, so this is 1 for a single-shot ingest; kept as
    /// a count so a future chunking ingest can report >1 without an API change.
    pub chunks: usize,
    /// Human-readable target the records landed in (the sidecar namespace).
    pub collection: String,
}

/// A retrieval query against long-term memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrieveQuery {
    pub query: String,
    pub top_k: u32,
}

/// One retrieved chunk — canonical text plus a rank-derived score.
///
/// The sidecar's `/recall` returns records with NO per-hit score over the wire
/// (dm-lite `Entry` has no score field), so the client synthesizes a
/// rank-derived score `1.0 - i/len` and stamps it here (a known fidelity gap,
/// documented in the plan).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedChunk {
    /// Canonical record uri (`daimon://…`). Stable across recalls.
    pub uri: String,
    pub source_id: String,
    pub source_kind: String,
    pub content: String,
    /// Rank-derived score in [0,1] (see struct doc).
    pub score: f32,
}

/// The typed-record kinds daimon captures from the AIOps loop. A strict subset
/// of dm-lite's `Kind` — the three the chat/orchestrator/triage paths write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Decision,
    Incident,
    Lesson,
}

impl RecordKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordKind::Decision => "decision",
            RecordKind::Incident => "incident",
            RecordKind::Lesson => "lesson",
        }
    }
}

/// The body of a typed record. The variants map 1:1 to dm-lite's typed-save
/// routes (`/log_decision`, `/log_incident`, `/log_lesson`); field names match
/// the corresponding request structs so the tool input_schema and the wire body
/// share one shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypedBody {
    Decision {
        title: String,
        #[serde(default)]
        context: String,
        decision: String,
        #[serde(default)]
        rationale: String,
    },
    Incident {
        title: String,
        impact: String,
        #[serde(default)]
        resolution: String,
    },
    Lesson {
        title: String,
        lesson: String,
    },
}

impl TypedBody {
    /// The kind discriminant for this body.
    pub fn kind(&self) -> RecordKind {
        match self {
            TypedBody::Decision { .. } => RecordKind::Decision,
            TypedBody::Incident { .. } => RecordKind::Incident,
            TypedBody::Lesson { .. } => RecordKind::Lesson,
        }
    }
}

/// A typed record to capture: a body plus an optional namespace override.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedRecord {
    #[serde(flatten)]
    pub body: TypedBody,
    /// Override the sidecar namespace. `None` = the kind's default namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

/// A scored typed record returned by [`MemoryService::recall`]. Wraps the
/// canonical uri + title + body text with a rank-derived score (same synthesis
/// as [`RetrievedChunk::score`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredRecord {
    pub uri: String,
    pub title: String,
    pub content: String,
    pub score: f32,
}

/// Budget for a pre-turn recall. Bounds how much recalled context may be packed
/// into a system-prompt addendum, and whether to rerank.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RecallBudget {
    /// Upper bound on packed context, in (estimated) tokens.
    pub max_tokens: usize,
    /// How many hits to pull per recall query before packing.
    pub top_k: usize,
    /// Whether the sidecar should rerank (advisory — the sidecar may ignore it).
    pub rerank: bool,
}

impl Default for RecallBudget {
    fn default() -> Self {
        RecallBudget {
            max_tokens: 3000,
            top_k: 6,
            rerank: true,
        }
    }
}

/// The assembled pre-turn context. `block` is the packed, human-readable
/// reference text (empty when nothing recalled); `degraded` is `true` whenever
/// the recall could not run to completion (backend fault, timeout, or the
/// `NullMemory` no-op) — the caller uses it to badge the turn without ever
/// failing it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PreTurnContext {
    pub block: String,
    pub degraded: bool,
    /// Number of source hits packed into `block` (0 when degraded/empty).
    pub hits: usize,
}

impl PreTurnContext {
    /// The fail-soft sentinel: no recalled context, flagged degraded. Returned by
    /// `pre_turn_recall` on ANY backend fault — that method NEVER returns `Err`.
    pub fn degraded() -> Self {
        PreTurnContext {
            block: String::new(),
            degraded: true,
            hits: 0,
        }
    }

    /// True when there is a non-empty recalled block to inject.
    pub fn has_context(&self) -> bool {
        !self.block.trim().is_empty()
    }
}

/// Liveness of the memory tier, surfaced by `/healthz` (self-observability).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryHealth {
    pub reachable: bool,
    /// Optional detail (endpoint, error class) for the ops surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The memory tier seam. One trait, two impls: [`crate::dmem_http::DmemHttpMemory`]
/// (the real sidecar client) and [`crate::dmem_http::NullMemory`] (a no-op used
/// when the sidecar is unconfigured/unreachable at boot, so memory absence
/// degrades rather than fails boot).
///
/// Write methods (`ingest`/`delete`/`capture`) return `Result` so the caller can
/// audit success/failure. Read methods used on the hot chat path
/// (`pre_turn_recall`) are contractually fail-soft — see the method docs.
#[async_trait]
pub trait MemoryService: Send + Sync {
    /// Ingest a document into long-term memory.
    async fn ingest(&self, doc: IngestDoc) -> Result<IngestStats>;

    /// Delete a record by its canonical uri.
    async fn delete(&self, uri: &str) -> Result<()>;

    /// Retrieve `top_k` chunks for a free-text query (the admin search path).
    async fn retrieve(&self, q: &RetrieveQuery) -> Result<Vec<RetrievedChunk>>;

    /// Capture a typed record (decision / incident / lesson).
    async fn capture(&self, rec: TypedRecord) -> Result<String>;

    /// Recall typed records semantically (used by triage / diagnostics).
    async fn recall(&self, query: &str, budget: RecallBudget) -> Result<Vec<ScoredRecord>>;

    /// Assemble a pre-turn context block for the chat system prompt.
    ///
    /// CONTRACT: this method NEVER returns `Err`. On ANY backend fault — connect
    /// refused, non-2xx, decode failure, or the null impl — it returns
    /// [`PreTurnContext::degraded`]. The chat handler wraps it in a timeout too,
    /// so recall can neither block nor fail a turn.
    async fn pre_turn_recall(&self, user_message: &str, budget: RecallBudget) -> PreTurnContext;

    /// Liveness probe for the memory tier.
    async fn health(&self) -> MemoryHealth;
}
