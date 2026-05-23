//! Shared trait + types for the working memory tier.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::error::Result;

/// One turn in a chat / orchestration conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvMessage {
    pub role: String,
    pub content: String,
    /// Optional tool-use binding (Phase 4 D3 chat surface uses this).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    pub ts: DateTime<Utc>,
}

/// Hot working memory tier — Redis-backed in prod, in-process in tests.
///
/// Keys are namespaced by category so different concerns coexist on a
/// shared Redis cluster:
/// - `conv:{session_id}` — list of ConvMessage (chat history)
/// - `kv:{agent_id}:{key}` — typed value with TTL (plan-in-flight, rate
///   limit counters, etc.)
/// - `signal:kill` — kill-switch channel (pub/sub, defence in depth)
#[async_trait]
pub trait WorkingMemory: Send + Sync {
    /// Append a conversation message to a session. Sessions are stored as a
    /// list keyed by session_id; the call is O(1).
    async fn conv_push(&self, session_id: &str, msg: ConvMessage) -> Result<()>;

    /// Read the last `n` messages of a conversation (most-recent-first or
    /// oldest-first depending on impl — Redis impl returns oldest-first to
    /// match the existing chat handler's expectation).
    async fn conv_recent(&self, session_id: &str, n: usize) -> Result<Vec<ConvMessage>>;

    /// Set a key with a TTL. `value` is serialized JSON.
    async fn kv_set(
        &self,
        agent_id: &str,
        key: &str,
        value: serde_json::Value,
        ttl: Duration,
    ) -> Result<()>;

    /// Get a typed value back. Returns None if missing or expired.
    async fn kv_get(&self, agent_id: &str, key: &str) -> Result<Option<serde_json::Value>>;

    /// Delete a key. Idempotent.
    async fn kv_delete(&self, agent_id: &str, key: &str) -> Result<()>;

    /// Publish a kill-switch signal. Phase 5 KILL switch consults filesystem
    /// flag AND subscribes to this channel.
    async fn kill_publish(&self, reason: &str) -> Result<()>;
}
