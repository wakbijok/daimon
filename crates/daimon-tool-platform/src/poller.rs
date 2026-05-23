//! Per-platform polling loop. Replaces the inline poll task that used to
//! live in `daimon-app/src/main.rs`.
//!
//! The poller calls `Platform::list_workloads()` on an interval, broadcasts
//! the JSON snapshot via a tokio broadcast channel (the existing WS
//! subscription path stays unchanged), and exposes the latest snapshot via
//! a shared cache for cold-start reads.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{RwLock, broadcast};
use tracing::{debug, error, info};

use crate::platform::{Platform, Workload};

#[derive(Debug, Clone, Copy)]
pub struct PollerConfig {
    pub interval: Duration,
}

impl Default for PollerConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
        }
    }
}

/// Snapshot held by the cache. Keyed by platform id.
#[derive(Debug, Default, Clone)]
pub struct PlatformCacheEntry {
    pub workloads: Vec<Workload>,
    pub last_poll: Option<Instant>,
}

/// Shared cache of latest snapshots, one entry per platform id.
#[derive(Default)]
pub struct PlatformCache {
    pub entries: RwLock<HashMap<String, PlatformCacheEntry>>,
}

impl PlatformCache {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Spawn a polling task per platform. The task takes ownership of the
/// platform Arc and runs until the runtime exits. Each poll fires a
/// broadcast event with the snapshot JSON (callers wire that to their
/// WebSocket scope).
///
/// Returns the broadcast sender so callers may subscribe + filter.
pub struct PlatformPoller {
    cache: Arc<PlatformCache>,
    tx: broadcast::Sender<String>,
}

impl PlatformPoller {
    pub fn new(buffer: usize) -> Self {
        let (tx, _rx) = broadcast::channel(buffer);
        Self {
            cache: Arc::new(PlatformCache::new()),
            tx,
        }
    }

    pub fn cache(&self) -> Arc<PlatformCache> {
        self.cache.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    pub fn sender(&self) -> broadcast::Sender<String> {
        self.tx.clone()
    }

    pub fn spawn(&self, platform: Arc<dyn Platform>, cfg: PollerConfig) {
        let cache = self.cache.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let id = platform.id().to_string();
            let mut interval = tokio::time::interval(cfg.interval);
            info!(platform = %id, kind = platform.kind(), "poller spawned");
            loop {
                interval.tick().await;
                match platform.list_workloads().await {
                    Ok(workloads) => {
                        // Update cache.
                        {
                            let mut guard = cache.entries.write().await;
                            guard.insert(
                                id.clone(),
                                PlatformCacheEntry {
                                    workloads: workloads.clone(),
                                    last_poll: Some(Instant::now()),
                                },
                            );
                        }
                        // Broadcast — caller wires to WS scope.
                        let payload = serde_json::json!({
                            "platform_id": id,
                            "kind": platform.kind(),
                            "workloads": workloads,
                        });
                        if let Ok(json) = serde_json::to_string(&payload) {
                            let _ = tx.send(json);
                        }
                        debug!(platform = %id, count = workloads.len(), "poll ok");
                    }
                    Err(e) => {
                        error!(platform = %id, error = %e, "poll failed");
                    }
                }
            }
        });
    }
}
