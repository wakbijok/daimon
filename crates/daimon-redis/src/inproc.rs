//! In-process `WorkingMemory` for tests + the no-Redis dev fallback.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value as Json;
use tokio::sync::RwLock;

use crate::error::Result;
use crate::traits::{ConvMessage, WorkingMemory};

#[derive(Default)]
struct Inner {
    conv: HashMap<String, Vec<ConvMessage>>,
    kv: HashMap<String, (Json, Option<Instant>)>,
}

#[derive(Default, Clone)]
pub struct InProcWorkingMemory {
    inner: Arc<RwLock<Inner>>,
}

impl InProcWorkingMemory {
    pub fn new() -> Self {
        Self::default()
    }

    fn kv_key(agent_id: &str, key: &str) -> String {
        format!("{agent_id}:{key}")
    }
}

#[async_trait]
impl WorkingMemory for InProcWorkingMemory {
    async fn conv_push(&self, session_id: &str, msg: ConvMessage) -> Result<()> {
        let mut guard = self.inner.write().await;
        guard
            .conv
            .entry(session_id.to_string())
            .or_default()
            .push(msg);
        Ok(())
    }

    async fn conv_recent(&self, session_id: &str, n: usize) -> Result<Vec<ConvMessage>> {
        let guard = self.inner.read().await;
        let list = guard.conv.get(session_id).cloned().unwrap_or_default();
        let start = list.len().saturating_sub(n);
        Ok(list[start..].to_vec())
    }

    async fn kv_set(
        &self,
        agent_id: &str,
        key: &str,
        value: Json,
        ttl: Duration,
    ) -> Result<()> {
        let mut guard = self.inner.write().await;
        let deadline = if ttl.is_zero() {
            None
        } else {
            Some(Instant::now() + ttl)
        };
        guard
            .kv
            .insert(Self::kv_key(agent_id, key), (value, deadline));
        Ok(())
    }

    async fn kv_get(&self, agent_id: &str, key: &str) -> Result<Option<Json>> {
        let now = Instant::now();
        let mut guard = self.inner.write().await;
        let k = Self::kv_key(agent_id, key);
        let expired = matches!(guard.kv.get(&k), Some((_, Some(d))) if *d < now);
        if expired {
            guard.kv.remove(&k);
        }
        Ok(guard.kv.get(&k).map(|(v, _)| v.clone()))
    }

    async fn kv_delete(&self, agent_id: &str, key: &str) -> Result<()> {
        let mut guard = self.inner.write().await;
        guard.kv.remove(&Self::kv_key(agent_id, key));
        Ok(())
    }

    async fn kill_publish(&self, _reason: &str) -> Result<()> {
        // In-process impl has no subscribers; the daimon-app KILL switch
        // (Phase 5) reads the filesystem flag for the in-proc dev path.
        Ok(())
    }
}
