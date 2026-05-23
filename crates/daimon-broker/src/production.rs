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
//! Phase 2c D3b: storage moved from SQLite to PostgreSQL. The boot now
//! requires a `pg_url` + `tenant_slug`; vault/inventory/audit are all
//! pool-backed against the relational tier owned by `daimon-db`. SSH
//! transport unchanged.
//!
//! Typical use from `daimon-app/src/main.rs`:
//!
//! ```text
//! let master_key = daimon_broker::production::MasterKey::from_systemd_or_dev_env()?;
//! let broker = daimon_broker::production::build_production_broker(BootConfig {
//!     pg_url: std::env::var("DAIMON_PG_URL")?,
//!     tenant_slug: "default".into(),
//!     known_hosts_path: "/var/lib/daimon/known_hosts".into(),
//!     master_key,
//! })
//! .await?;
//! ```
//!
//! See `daimon-docs/specs/2026-05-20-multi-agent-architecture-design.md` D21
//! / D22 / D23 / D24 and `daimon-docs/MASTERPLAN.md` §5.2.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use daimon_audit::{AuditSink, PostgresAuditSink};
use daimon_inventory::{Inventory, PostgresRegistry};
use daimon_transport::{SshTransport, Transport};
use daimon_vault::{MasterKey, PostgresVaultClient};
use thiserror::Error;
use tracing::info;
use uuid::Uuid;

use crate::{Broker, TransportKind};

/// Re-export so `daimon-app` can load the key without depending on
/// `daimon-vault` directly (D21).
pub use daimon_vault::{MasterKey as MasterKeyHandle, MasterKeyError};

/// Boot configuration for the production broker stack.
pub struct BootConfig {
    /// PostgreSQL connection URL. Typically derived from `$DAIMON_PG_URL`.
    pub pg_url: String,
    /// Tenant slug this broker instance serves. D6 will plumb multi-tenant
    /// routing; for now the broker is single-tenant per process.
    pub tenant_slug: String,
    /// SSH `known_hosts` file path. Production should be
    /// `/var/lib/daimon/known_hosts`. The file does not need to exist at
    /// startup — it is created on first SSH connect or by an explicit
    /// `--learn-known-hosts` bootstrap flow.
    pub known_hosts_path: PathBuf,
    /// Master key for the in-tree vault. Caller is responsible for loading
    /// it via `MasterKey::from_systemd_or_dev_env()` or equivalent.
    pub master_key: MasterKey,
    /// Phase 5 — path to the KILL switch file. Operator `touch`es to
    /// engage; `rm`s to resume. Default `<data_dir>/KILL`.
    pub kill_path: PathBuf,
    /// Phase 5 — path to the policy TOML. Missing file = default-deny
    /// engine (every write rejected until a policy is written).
    pub policy_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum BootError {
    #[error("db: {0}")]
    Db(#[from] daimon_db::Error),
    #[error("pg client: {0}")]
    PgClient(#[from] tokio_postgres::Error),
    #[error("pool: {0}")]
    Pool(String),
    #[error("tenant `{0}` not found in public.tenants")]
    TenantNotFound(String),
    #[error("vault: {0}")]
    Vault(#[from] daimon_vault::VaultError),
    #[error("inventory: {0}")]
    Inventory(#[from] daimon_inventory::InventoryError),
    #[error("audit: {0}")]
    Audit(#[from] daimon_audit::AuditError),
    #[error("guard: {0}")]
    Guard(#[from] daimon_guard::Error),
}

/// Assemble the production broker. Opens a Postgres pool, resolves the
/// tenant id, and wires the russh-backed SSH transport against
/// `cfg.known_hosts_path`. Returns the broker wrapped in `Arc` for sharing
/// across request handlers.
pub async fn build_production_broker(cfg: BootConfig) -> Result<Arc<Broker>, BootError> {
    info!(
        pg_url = %scrub_pg_url(&cfg.pg_url),
        tenant = %cfg.tenant_slug,
        known_hosts = %cfg.known_hosts_path.display(),
        "assembling production broker stack"
    );

    let pool = daimon_db::build_pool(&cfg.pg_url)?;
    let tenant_id = resolve_tenant(&pool, &cfg.tenant_slug).await?;

    let vault = Arc::new(PostgresVaultClient::new(pool.clone(), tenant_id, cfg.master_key));
    let inventory: Arc<dyn Inventory> = Arc::new(PostgresRegistry::new(pool.clone(), tenant_id));
    let audit: Arc<dyn AuditSink> = Arc::new(PostgresAuditSink::new(pool.clone(), tenant_id));

    let ssh: Arc<dyn Transport> =
        Arc::new(SshTransport::with_known_hosts_path(cfg.known_hosts_path));
    let mut transports: HashMap<TransportKind, Arc<dyn Transport>> = HashMap::new();
    transports.insert(TransportKind::Ssh, ssh);

    // Phase 5 — assemble Guard.
    let kill_switch = daimon_guard::KillSwitch::new(cfg.kill_path.clone());
    kill_switch.spawn_watchers();
    let policy = daimon_guard::PolicyEngine::from_toml_file(&cfg.policy_path)?;
    let approvals = daimon_guard::ApprovalQueue::new(pool.clone());
    let guard = Arc::new(daimon_guard::Guard::new(
        kill_switch.state(),
        policy,
        approvals,
        tenant_id,
    ));
    info!(
        kill_path = %cfg.kill_path.display(),
        policy_path = %cfg.policy_path.display(),
        rules = guard.policy().rules().len(),
        "guard ready"
    );

    let broker = Broker::with_production_admin(inventory, vault, audit, transports).with_guard(guard);
    Ok(Arc::new(broker))
}

async fn resolve_tenant(pool: &daimon_db::Pool, slug: &str) -> Result<Uuid, BootError> {
    let client = pool.get().await.map_err(|e| BootError::Pool(e.to_string()))?;
    let row = client
        .query_opt("SELECT id FROM public.tenants WHERE slug = $1", &[&slug])
        .await?
        .ok_or_else(|| BootError::TenantNotFound(slug.to_string()))?;
    Ok(row.get(0))
}

fn scrub_pg_url(url: &str) -> String {
    if let Some(at) = url.rfind('@') {
        if let Some(scheme_end) = url.find("://") {
            let scheme = &url[..scheme_end + 3];
            let after_at = &url[at..];
            return format!("{scheme}*****{after_at}");
        }
    }
    url.to_string()
}
