//! Production stack assembler — used by `daimon-app` at boot to build the
//! full broker (vault + inventory + transport + audit) without depending on
//! the internal crates directly.
//!
//! Per D21, only `daimon-broker` and `daimon-cli` are wrapper crates that
//! may import `daimon-vault`, `daimon-inventory`, and `daimon-transport`
//! directly. The long-running I/O adapter (`daimon-app`) goes through this
//! module instead, so the architectural single-integration-point invariant
//! holds.
//!
//! Typical use from `daimon-app/src/main.rs`:
//!
//! ```text
//! let master_key = daimon_broker::production::MasterKey::from_systemd_or_dev_env()?;
//! let broker = daimon_broker::production::build_production_broker(BootConfig {
//!     data_dir: "/var/lib/daimon".into(),
//!     known_hosts_path: "/var/lib/daimon/known_hosts".into(),
//!     master_key,
//! })
//! .await?;
//! ```
//!
//! See `docs/specs/2026-05-20-multi-agent-architecture-design.md` D21 / D22 /
//! D23 / D24.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use daimon_audit::{AuditSink, SqliteAuditSink};
use daimon_inventory::{Inventory, SqliteRegistry};
use daimon_transport::{SshTransport, Transport};
use daimon_vault::{MasterKey, SqliteVaultClient};
use thiserror::Error;
use tracing::info;

use crate::{Broker, TransportKind};

/// Re-export so `daimon-app` can load the key without depending on
/// `daimon-vault` directly (D21).
pub use daimon_vault::{MasterKey as MasterKeyHandle, MasterKeyError};

/// Boot configuration for the production broker stack.
///
/// Filesystem paths are caller-owned — `daimon-app` typically derives them
/// from `DAIMON_DATA_DIR` + `DAIMON_KNOWN_HOSTS_PATH` env vars at start.
pub struct BootConfig {
    /// Directory holding `vault.db`, `inventory.db`, `audit.db`. Created if
    /// missing.
    pub data_dir: PathBuf,
    /// SSH `known_hosts` file path. Production should be
    /// `/var/lib/daimon/known_hosts`. The file does not need to exist at
    /// startup — it is created on first SSH connect or by an explicit
    /// `--learn-known-hosts` bootstrap flow.
    pub known_hosts_path: PathBuf,
    /// Master key for the in-tree vault. Caller is responsible for loading
    /// it via `MasterKey::from_systemd_or_dev_env()` or equivalent.
    pub master_key: MasterKey,
}

#[derive(Debug, Error)]
pub enum BootError {
    #[error("io: {context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("vault: {0}")]
    Vault(#[from] daimon_vault::VaultError),
    #[error("inventory: {0}")]
    Inventory(#[from] daimon_inventory::InventoryError),
    #[error("audit: {0}")]
    Audit(#[from] daimon_audit::AuditError),
}

/// Assemble the production broker. Opens (or creates) the three SQLite
/// databases inside `cfg.data_dir`, wires the russh-backed SSH transport
/// against `cfg.known_hosts_path`, and returns the broker wrapped in `Arc`
/// for sharing across request handlers.
///
/// SSH transport defaults to `KnownHosts` policy — first-connect failures
/// against unknown hosts surface as an explicit `TransportError`, never
/// silent trust. Operators bootstrap a new host via `daimon-cli`'s
/// `--learn-known-hosts` flag.
pub async fn build_production_broker(cfg: BootConfig) -> Result<Arc<Broker>, BootError> {
    std::fs::create_dir_all(&cfg.data_dir).map_err(|e| BootError::Io {
        context: format!("create data_dir `{}`", cfg.data_dir.display()),
        source: e,
    })?;

    let vault_path = cfg.data_dir.join("vault.db");
    let inventory_path = cfg.data_dir.join("inventory.db");
    let audit_path = cfg.data_dir.join("audit.db");

    info!(
        data_dir = %cfg.data_dir.display(),
        known_hosts = %cfg.known_hosts_path.display(),
        "assembling production broker stack"
    );

    let vault = Arc::new(SqliteVaultClient::open(&vault_path, cfg.master_key).await?);
    let inventory: Arc<dyn Inventory> = Arc::new(SqliteRegistry::open(&inventory_path).await?);
    let audit: Arc<dyn AuditSink> = Arc::new(SqliteAuditSink::open(&audit_path).await?);

    let ssh: Arc<dyn Transport> =
        Arc::new(SshTransport::with_known_hosts_path(cfg.known_hosts_path));
    let mut transports: HashMap<TransportKind, Arc<dyn Transport>> = HashMap::new();
    transports.insert(TransportKind::Ssh, ssh);

    let broker = Broker::with_production_admin(inventory, vault, audit, transports);
    Ok(Arc::new(broker))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn build_production_broker_opens_all_three_dbs_in_fresh_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = BootConfig {
            data_dir: tmp.path().to_path_buf(),
            known_hosts_path: tmp.path().join("known_hosts"),
            master_key: MasterKey::from_bytes([0xC3u8; 32]),
        };

        let broker = build_production_broker(cfg).await.expect("boot");

        // Sanity: broker is reachable as an admin proxy and returns empty
        // results on a fresh stack.
        let targets = broker.list_targets(None).await;
        assert!(targets.is_empty(), "fresh inventory should be empty");

        // Three DB files should exist on disk.
        assert!(tmp.path().join("vault.db").exists());
        assert!(tmp.path().join("inventory.db").exists());
        assert!(tmp.path().join("audit.db").exists());
    }

    #[tokio::test]
    async fn build_production_broker_creates_missing_data_dir() {
        let parent = tempfile::tempdir().unwrap();
        let data_dir = parent.path().join("nested/dir");
        let cfg = BootConfig {
            data_dir: data_dir.clone(),
            known_hosts_path: data_dir.join("known_hosts"),
            master_key: MasterKey::from_bytes([0x55u8; 32]),
        };

        let _broker = build_production_broker(cfg).await.expect("boot");
        assert!(data_dir.exists(), "data_dir should be created on boot");
    }
}
