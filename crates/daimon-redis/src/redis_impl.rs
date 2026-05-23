//! Redis-backed `WorkingMemory` implementation. Pool via deadpool-redis.

use std::time::Duration;

use async_trait::async_trait;
use deadpool_redis::{Config, Pool, Runtime};
use deadpool_redis::redis::AsyncCommands;
use serde_json::Value as Json;

use crate::error::{Error, Result};
use crate::traits::{ConvMessage, WorkingMemory};

const CONV_KEY_PREFIX: &str = "daimon:conv:";
const KV_KEY_PREFIX: &str = "daimon:kv:";
const KILL_CHANNEL: &str = "daimon:signal:kill";

#[derive(Clone)]
pub struct RedisWorkingMemory {
    pool: Pool,
}

impl RedisWorkingMemory {
    /// Connect to a redis instance — typically `redis://localhost:6379`.
    pub fn from_url(url: &str) -> Result<Self> {
        let cfg = Config::from_url(url);
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1))
            .map_err(|e| Error::Pool(e.to_string()))?;
        Ok(Self { pool })
    }

    fn conv_key(session_id: &str) -> String {
        format!("{CONV_KEY_PREFIX}{session_id}")
    }

    fn kv_key(agent_id: &str, key: &str) -> String {
        format!("{KV_KEY_PREFIX}{agent_id}:{key}")
    }
}

#[async_trait]
impl WorkingMemory for RedisWorkingMemory {
    async fn conv_push(&self, session_id: &str, msg: ConvMessage) -> Result<()> {
        let mut conn = self.pool.get().await?;
        let key = Self::conv_key(session_id);
        let payload = serde_json::to_string(&msg)
            .map_err(|e| Error::Decode(format!("conv encode: {e}")))?;
        // RPUSH appends; we read with LRANGE so oldest-first order is preserved.
        let _: () = conn.rpush(&key, payload).await?;
        // Conversation expires after 24h of inactivity to keep Redis tidy.
        let _: () = conn.expire(&key, 86400).await?;
        Ok(())
    }

    async fn conv_recent(&self, session_id: &str, n: usize) -> Result<Vec<ConvMessage>> {
        let mut conn = self.pool.get().await?;
        let key = Self::conv_key(session_id);
        let len: i64 = conn.llen(&key).await?;
        let start: isize = (len as isize - n as isize).max(0);
        let raw: Vec<String> = conn.lrange(&key, start, -1).await?;
        raw.into_iter()
            .map(|s| {
                serde_json::from_str::<ConvMessage>(&s)
                    .map_err(|e| Error::Decode(format!("conv decode: {e}")))
            })
            .collect()
    }

    async fn kv_set(
        &self,
        agent_id: &str,
        key: &str,
        value: Json,
        ttl: Duration,
    ) -> Result<()> {
        let mut conn = self.pool.get().await?;
        let k = Self::kv_key(agent_id, key);
        let payload = serde_json::to_string(&value)
            .map_err(|e| Error::Decode(format!("kv encode: {e}")))?;
        let secs = ttl.as_secs().max(1) as i64;
        let _: () = conn.set_ex(&k, payload, secs as u64).await?;
        Ok(())
    }

    async fn kv_get(&self, agent_id: &str, key: &str) -> Result<Option<Json>> {
        let mut conn = self.pool.get().await?;
        let k = Self::kv_key(agent_id, key);
        let raw: Option<String> = conn.get(&k).await?;
        match raw {
            Some(s) => Ok(Some(
                serde_json::from_str(&s)
                    .map_err(|e| Error::Decode(format!("kv decode: {e}")))?,
            )),
            None => Ok(None),
        }
    }

    async fn kv_delete(&self, agent_id: &str, key: &str) -> Result<()> {
        let mut conn = self.pool.get().await?;
        let k = Self::kv_key(agent_id, key);
        let _: i64 = conn.del(&k).await?;
        Ok(())
    }

    async fn kill_publish(&self, reason: &str) -> Result<()> {
        let mut conn = self.pool.get().await?;
        let _: i64 = conn.publish(KILL_CHANNEL, reason).await?;
        Ok(())
    }
}
