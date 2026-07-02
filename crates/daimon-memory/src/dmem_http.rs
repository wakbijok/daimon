//! The dmem sidecar HTTP client (`DmemHttpMemory`) + a no-op [`NullMemory`].
//!
//! P3 LOCKED: daimon talks to a running `dmem serve` (dm-lite HTTP server) over
//! bearer-authenticated HTTP. This module is the ONLY place `reqwest` is used;
//! it is compiled ssr-side only (the trait + DTOs in [`crate::service`] carry
//! the wire shapes to the wasm/hydrate side without pulling `reqwest` in).
//!
//! Route mapping (dm-lite `src/server.rs`):
//! | trait method        | route            | notes                                   |
//! |---------------------|------------------|-----------------------------------------|
//! | `ingest`            | POST /remember   | text = content, namespace from kind     |
//! | `delete`            | POST /forget     | { uri }                                 |
//! | `retrieve`          | POST /recall     | { query, limit } → Vec<Entry>           |
//! | `capture` (Decision)| POST /log_decision |                                       |
//! | `capture` (Incident)| POST /log_incident |                                       |
//! | `capture` (Lesson)  | POST /log_lesson |                                         |
//! | `recall`            | POST /recall     | filtered to typed kinds                 |
//! | `pre_turn_recall`   | POST /recall ×2  | docs + typed, rank-greedy pack (no Err) |
//! | `health`            | GET  /healthz    |                                         |
//!
//! dm-lite `Entry` has NO per-hit score over the wire, so recall scores are
//! synthesized rank-derived (`1.0 - i/len`).

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::error::{Error, Result};
use crate::service::{
    IngestDoc, IngestStats, MemoryHealth, MemoryService, PreTurnContext, RecallBudget, RecordKind,
    RetrieveQuery, RetrievedChunk, ScoredRecord, TypedBody, TypedRecord,
};

/// dm-lite's `Entry` response shape (the fields we read). Deserialized from
/// `/recall`. Extra fields (bitemporal columns, tags, importance) are ignored.
#[derive(Debug, Clone, Deserialize)]
struct DmEntry {
    uri: String,
    kind: String,
    namespace: String,
    title: String,
    body: String,
}

/// Per-operation timeouts. Writes get a little more room than reads; the hot
/// pre-turn recall is additionally wrapped in a caller-side timeout.
const READ_TIMEOUT: Duration = Duration::from_secs(3);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);

/// The dm-lite kinds that count as "typed records" for the AIOps loop. Recall
/// filters recalled entries to these to separate typed knowledge from raw docs.
fn is_typed_kind(kind: &str) -> bool {
    matches!(
        kind,
        "decision" | "incident_summary" | "incident" | "agent_lesson" | "lesson"
    )
}

/// Rough token estimate (~4 chars/token) for packing under a token budget
/// without a tokenizer dependency.
fn est_tokens(s: &str) -> usize {
    (s.chars().count() / 4).max(1)
}

/// The sidecar client. Cheap to clone (`reqwest::Client` is an `Arc` internally,
/// but we hold one and share via `Arc<dyn MemoryService>`).
pub struct DmemHttpMemory {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl DmemHttpMemory {
    /// Build a client for `base` (e.g. `http://localhost:7071`) authenticating
    /// with `token`. Errors only if the reqwest client cannot be constructed.
    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| Error::Http(format!("build reqwest client: {e}")))?;
        let base = base.into();
        let base = base.trim_end_matches('/').to_string();
        Ok(Self {
            base,
            token: token.into(),
            http,
        })
    }

    /// POST `path` with a JSON `body`, bearer-authenticated, bounded by `timeout`.
    /// Non-2xx → [`Error::Http`]; a body that will not decode → [`Error::Decode`];
    /// a transport failure (connect refused, DNS, TLS) → [`Error::Unreachable`].
    async fn post<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: serde_json::Value,
        timeout: Duration,
    ) -> Result<T> {
        let url = format!("{}{}", self.base, path);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .json(&body)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| Error::Unreachable(format!("POST {path}: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let detail = resp.text().await.unwrap_or_default();
            let detail = detail.chars().take(200).collect::<String>();
            return Err(Error::Http(format!("POST {path} -> {status}: {detail}")));
        }
        resp.json::<T>()
            .await
            .map_err(|e| Error::Decode(format!("decode {path} response: {e}")))
    }

    /// Raw `/recall` returning dm-lite entries (shared by `retrieve`, `recall`,
    /// and `pre_turn_recall`).
    async fn recall_entries(&self, query: &str, limit: usize) -> Result<Vec<DmEntry>> {
        self.post(
            "/recall",
            json!({ "query": query, "limit": limit }),
            READ_TIMEOUT,
        )
        .await
    }
}

/// Synthesize a rank-derived score for hit `i` of `len` total: `1.0 - i/len`,
/// so the top hit scores ~1.0 and the last scores just above 0.
fn rank_score(i: usize, len: usize) -> f32 {
    if len == 0 {
        return 0.0;
    }
    1.0 - (i as f32) / (len as f32)
}

#[async_trait]
impl MemoryService for DmemHttpMemory {
    async fn ingest(&self, doc: IngestDoc) -> Result<IngestStats> {
        // A document is remembered as one record; provenance (source id/kind) is
        // folded into the namespace so retrieval can attribute it back.
        let namespace = format!("resources/{}", doc.source_kind);
        let text = format!("[{}] {}", doc.source_id, doc.content);
        #[derive(Deserialize)]
        struct RememberResp {
            #[allow(dead_code)]
            uri: String,
        }
        let _resp: RememberResp = self
            .post(
                "/remember",
                json!({ "text": text, "namespace": namespace }),
                WRITE_TIMEOUT,
            )
            .await?;
        Ok(IngestStats {
            source_id: doc.source_id,
            chunks: 1,
            collection: namespace,
        })
    }

    async fn delete(&self, uri: &str) -> Result<()> {
        #[derive(Deserialize)]
        struct ForgetResp {
            #[allow(dead_code)]
            forgotten: bool,
        }
        let _r: ForgetResp = self
            .post("/forget", json!({ "uri": uri }), WRITE_TIMEOUT)
            .await?;
        Ok(())
    }

    async fn retrieve(&self, q: &RetrieveQuery) -> Result<Vec<RetrievedChunk>> {
        let entries = self.recall_entries(&q.query, q.top_k as usize).await?;
        let len = entries.len();
        Ok(entries
            .into_iter()
            .enumerate()
            .map(|(i, e)| RetrievedChunk {
                uri: e.uri,
                source_id: e.namespace,
                source_kind: e.kind,
                content: e.body,
                score: rank_score(i, len),
            })
            .collect())
    }

    async fn capture(&self, rec: TypedRecord) -> Result<String> {
        #[derive(Deserialize)]
        struct UriResp {
            uri: String,
        }
        let (path, mut payload) = match &rec.body {
            TypedBody::Decision {
                title,
                context,
                decision,
                rationale,
            } => (
                "/log_decision",
                json!({
                    "title": title,
                    "context": context,
                    "decision": decision,
                    "rationale": rationale,
                }),
            ),
            TypedBody::Incident {
                title,
                impact,
                resolution,
            } => (
                "/log_incident",
                json!({ "title": title, "impact": impact, "resolution": resolution }),
            ),
            TypedBody::Lesson { title, lesson } => {
                ("/log_lesson", json!({ "title": title, "lesson": lesson }))
            }
        };
        if let Some(ns) = &rec.namespace {
            payload["namespace"] = json!(ns);
        }
        let resp: UriResp = self.post(path, payload, WRITE_TIMEOUT).await?;
        Ok(resp.uri)
    }

    async fn recall(&self, query: &str, budget: RecallBudget) -> Result<Vec<ScoredRecord>> {
        // Over-fetch, then keep only typed-kind records (recall for typed
        // knowledge, not raw doc chunks).
        let entries = self
            .recall_entries(query, budget.top_k.max(1) * 2)
            .await?;
        let typed: Vec<DmEntry> = entries
            .into_iter()
            .filter(|e| is_typed_kind(&e.kind))
            .take(budget.top_k.max(1))
            .collect();
        let len = typed.len();
        Ok(typed
            .into_iter()
            .enumerate()
            .map(|(i, e)| ScoredRecord {
                uri: e.uri,
                title: e.title,
                content: e.body,
                score: rank_score(i, len),
            })
            .collect())
    }

    async fn pre_turn_recall(&self, user_message: &str, budget: RecallBudget) -> PreTurnContext {
        // CONTRACT: never Err. One /recall over everything (docs) + one filtered
        // to typed kinds, packed rank-greedy under the token budget. Any fault at
        // any step short-circuits to degraded().
        let limit = budget.top_k.max(1);
        let all = match self.recall_entries(user_message, limit).await {
            Ok(v) => v,
            Err(_) => return PreTurnContext::degraded(),
        };
        // A second recall biased to typed kinds. If it faults, degrade rather
        // than partially proceed (keeps the contract simple + predictable).
        let typed_pool = match self.recall_entries(user_message, limit * 2).await {
            Ok(v) => v,
            Err(_) => return PreTurnContext::degraded(),
        };

        // Merge: typed records first (higher-signal), then doc chunks, de-duped
        // by uri, each with a rank-derived score by original position.
        let typed: Vec<(f32, DmEntry)> = {
            let filtered: Vec<DmEntry> =
                typed_pool.into_iter().filter(|e| is_typed_kind(&e.kind)).collect();
            let len = filtered.len();
            filtered
                .into_iter()
                .enumerate()
                .map(|(i, e)| (rank_score(i, len), e))
                .collect()
        };
        let docs: Vec<(f32, DmEntry)> = {
            let len = all.len();
            all.into_iter()
                .enumerate()
                .map(|(i, e)| (rank_score(i, len), e))
                .collect()
        };

        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut packed: Vec<String> = Vec::new();
        let mut budget_left = budget.max_tokens;
        let mut hits = 0usize;
        for (_score, e) in typed.into_iter().chain(docs.into_iter()) {
            if !seen.insert(e.uri.clone()) {
                continue;
            }
            let line = format!("- ({}) {}: {}", e.kind, e.title, e.body);
            let cost = est_tokens(&line);
            if cost > budget_left {
                // Rank-greedy: stop at the first hit that would overflow.
                break;
            }
            budget_left -= cost;
            packed.push(line);
            hits += 1;
        }

        PreTurnContext {
            block: packed.join("\n"),
            degraded: false,
            hits,
        }
    }

    async fn health(&self) -> MemoryHealth {
        let url = format!("{}/healthz", self.base);
        match self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .timeout(HEALTH_TIMEOUT)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => MemoryHealth {
                reachable: true,
                detail: None,
            },
            Ok(r) => MemoryHealth {
                reachable: false,
                detail: Some(format!("healthz -> {}", r.status())),
            },
            Err(e) => MemoryHealth {
                reachable: false,
                detail: Some(format!("healthz unreachable: {e}")),
            },
        }
    }
}

/// A no-op memory tier used when the sidecar is unconfigured or was unreachable
/// at boot. Reads return empty / degraded; writes are `Ok`-swallowed (memory is
/// an aid, never a hard dependency). `health.reachable == false`.
pub struct NullMemory;

#[async_trait]
impl MemoryService for NullMemory {
    async fn ingest(&self, doc: IngestDoc) -> Result<IngestStats> {
        Ok(IngestStats {
            source_id: doc.source_id,
            chunks: 0,
            collection: "(null-memory)".into(),
        })
    }

    async fn delete(&self, _uri: &str) -> Result<()> {
        Ok(())
    }

    async fn retrieve(&self, _q: &RetrieveQuery) -> Result<Vec<RetrievedChunk>> {
        Ok(Vec::new())
    }

    async fn capture(&self, rec: TypedRecord) -> Result<String> {
        // Swallow — return a synthetic sentinel uri so callers can audit "wrote
        // to null memory" without surfacing an error to the operator.
        let kind = match rec.body.kind() {
            RecordKind::Decision => "decision",
            RecordKind::Incident => "incident",
            RecordKind::Lesson => "lesson",
        };
        Ok(format!("daimon://null-memory/{kind}"))
    }

    async fn recall(&self, _query: &str, _budget: RecallBudget) -> Result<Vec<ScoredRecord>> {
        Ok(Vec::new())
    }

    async fn pre_turn_recall(&self, _user_message: &str, _budget: RecallBudget) -> PreTurnContext {
        PreTurnContext::degraded()
    }

    async fn health(&self) -> MemoryHealth {
        MemoryHealth {
            reachable: false,
            detail: Some("null memory (sidecar unconfigured or unreachable at boot)".into()),
        }
    }
}
