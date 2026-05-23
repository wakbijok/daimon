//! KILL switch (D13).
//!
//! Two trigger surfaces:
//! 1. **Filesystem flag** — operator touches `$DAIMON_DATA_DIR/KILL` (or
//!    a configured absolute path). A background poll task (1s interval)
//!    flips an atomic bool ON when the file exists; flips it OFF when the
//!    file is removed.
//! 2. **SIGUSR1** — `kill -USR1 <daimon-app-pid>` flips the bool ON. The
//!    daemon ALSO creates the file so the operator must explicitly `rm` to
//!    clear (no auto-resume on next signal).
//!
//! `KillState::engaged()` is the cheap check broker.execute makes before
//! dispatching transport.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::watch;
use tracing::{info, warn};

/// Shared, cheaply-cloned kill-state handle.
#[derive(Clone)]
pub struct KillState {
    engaged: Arc<AtomicBool>,
    reason: Arc<watch::Sender<String>>,
}

impl KillState {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(String::new());
        Self {
            engaged: Arc::new(AtomicBool::new(false)),
            reason: Arc::new(tx),
        }
    }

    pub fn engaged(&self) -> bool {
        self.engaged.load(Ordering::Acquire)
    }

    pub fn reason(&self) -> String {
        self.reason.borrow().clone()
    }

    pub fn engage(&self, reason: impl Into<String>) {
        let r = reason.into();
        self.engaged.store(true, Ordering::Release);
        let _ = self.reason.send(r.clone());
        warn!(reason = %r, "KILL switch ENGAGED");
    }

    pub fn release(&self) {
        self.engaged.store(false, Ordering::Release);
        let _ = self.reason.send(String::new());
        info!("KILL switch released");
    }
}

impl Default for KillState {
    fn default() -> Self {
        Self::new()
    }
}

/// The KillSwitch owns the background tasks (file watcher + signal handler)
/// and exposes a shared `KillState` for the broker.
pub struct KillSwitch {
    state: KillState,
    path: PathBuf,
}

impl KillSwitch {
    pub fn new(path: PathBuf) -> Self {
        Self {
            state: KillState::new(),
            path,
        }
    }

    pub fn state(&self) -> KillState {
        self.state.clone()
    }

    /// Spawn the background watcher tasks. Returns immediately. Drops are
    /// not joined — the daemon assumes the runtime outlives them.
    pub fn spawn_watchers(&self) {
        let state = self.state.clone();
        let path = self.path.clone();
        tokio::spawn(async move {
            let mut last = false;
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let exists = tokio::fs::try_exists(&path).await.unwrap_or(false);
                if exists != last {
                    if exists {
                        let reason = tokio::fs::read_to_string(&path)
                            .await
                            .unwrap_or_else(|_| "file present".into())
                            .trim()
                            .to_string();
                        let r = if reason.is_empty() {
                            format!("KILL file present at {}", path.display())
                        } else {
                            reason
                        };
                        state.engage(r);
                    } else {
                        state.release();
                    }
                    last = exists;
                }
            }
        });

        #[cfg(unix)]
        {
            let state = self.state.clone();
            let path = self.path.clone();
            tokio::spawn(async move {
                let mut sig = match tokio::signal::unix::signal(
                    tokio::signal::unix::SignalKind::user_defined1(),
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(error = %e, "failed to register SIGUSR1 handler");
                        return;
                    }
                };
                loop {
                    sig.recv().await;
                    // Create the KILL file so the operator must rm it to resume.
                    if let Err(e) = tokio::fs::write(&path, "engaged via SIGUSR1\n").await {
                        warn!(error = %e, "failed to write KILL file on SIGUSR1");
                    }
                    state.engage("SIGUSR1 received");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn engage_and_release_round_trip() {
        let s = KillState::new();
        assert!(!s.engaged());
        s.engage("test");
        assert!(s.engaged());
        assert_eq!(s.reason(), "test");
        s.release();
        assert!(!s.engaged());
    }
}
