//! daimon-db — relational tier (Phase 2c).
//!
//! Owns the PostgreSQL schema for all of daimon's structured state:
//! - `public` — tenants, users, roles, role_grants, plan_history, clusters
//! - `vault` — credentials (encryption-version columns; ciphertext blobs)
//! - `inventory` — managed targets
//! - `audit` — append-only event log with hash chain (Phase 2c upgrade)
//!
//! Migrations live in `migrations/`, applied in lexicographic order by refinery.
//! Apply via `daimon-migrate` CLI or `daimon_db::run_migrations(&pg_url).await`.
//!
//! Connection management is `deadpool-postgres` (tokio-postgres pool). App
//! services receive a `&Pool` at construction; per-tenant scoping is enforced
//! at the SQL layer via row-level security policies (added in V002+).

pub mod error;

pub use deadpool_postgres::Pool;
pub use error::{Error, Result};

use deadpool_postgres::{Manager, ManagerConfig, RecyclingMethod};
use tokio_postgres::{Config, NoTls};

/// refinery-managed migration set. Bundled at compile time from `migrations/`.
mod embedded {
    refinery::embed_migrations!("./migrations");
}

/// Build a tokio-postgres `Config` from a `postgres://` URL.
fn parse_url(url: &str) -> Result<Config> {
    url.parse::<Config>().map_err(Error::from)
}

/// Build a deadpool connection pool to Postgres.
pub fn build_pool(url: &str) -> Result<Pool> {
    let cfg = parse_url(url)?;
    let mgr_cfg = ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    };
    let mgr = Manager::from_config(cfg, NoTls, mgr_cfg);
    let pool = Pool::builder(mgr)
        .max_size(20)
        .build()
        .map_err(|e| Error::Pool(format!("{e}")))?;
    Ok(pool)
}

/// Apply all pending migrations against the given Postgres URL. Opens its own
/// dedicated connection (migrations need exclusive access).
pub async fn run_migrations(url: &str) -> Result<()> {
    let cfg = parse_url(url)?;
    let (mut client, conn) = cfg.connect(NoTls).await?;
    let handle = tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::error!(error = %e, "pg conn task ended with error");
        }
    });
    let report = embedded::migrations::runner()
        .run_async(&mut client)
        .await?;
    drop(client);
    let _ = handle.await;
    for applied in report.applied_migrations() {
        tracing::info!(version = %applied.version(), name = %applied.name(), "migration applied");
    }
    Ok(())
}
