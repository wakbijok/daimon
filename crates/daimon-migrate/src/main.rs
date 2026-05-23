//! daimon-migrate-data — dev-only one-shot migrate: SQLite → PostgreSQL.
//!
//! Reads from the four Phase-2b SQLite stores (vault.db, inventory.db,
//! audit.db, daimon.db) and writes to the Phase-2c PostgreSQL schemas.
//! Idempotent: re-running upserts or skips dupes by deterministic key.
//!
//! Subcommands:
//!   run [--dry-run]         Migrate everything (vault, inventory, audit, app)
//!   vault [--dry-run]       Vault only
//!   inventory [--dry-run]   Inventory only
//!   audit [--dry-run]       Audit only (per-tenant hash chain reconstructed
//!                           by V008 trigger as rows insert in ts ASC order)
//!   app [--dry-run]         daimon.db (users, sessions, clusters, config,
//!                           user_preferences)
//!   verify                  Row counts side-by-side for each table
//!
//! Defaults:
//!   --sqlite-dir   $DAIMON_DATA_DIR or ./daimon-data
//!   --app-db       ./daimon.db (relative to cwd, matches daimon-app default)
//!   --pg-url       $DAIMON_PG_URL or postgres://$USER@localhost:5432/daimon
//!   --tenant-slug  default

mod app;
mod audit;
mod inventory;
mod vault;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "daimon-migrate-data", about = "SQLite → Postgres one-shot data migrate")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,

    #[arg(long, env = "DAIMON_PG_URL")]
    pg_url: Option<String>,

    #[arg(long, env = "DAIMON_DATA_DIR")]
    sqlite_dir: Option<PathBuf>,

    #[arg(long, default_value = "daimon.db")]
    app_db: PathBuf,

    #[arg(long, default_value = "default")]
    tenant_slug: String,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    Run {
        #[arg(long)]
        dry_run: bool,
    },
    Vault {
        #[arg(long)]
        dry_run: bool,
    },
    Inventory {
        #[arg(long)]
        dry_run: bool,
    },
    Audit {
        #[arg(long)]
        dry_run: bool,
    },
    App {
        #[arg(long)]
        dry_run: bool,
    },
    Verify,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MigrateStats {
    pub read: usize,
    pub inserted: usize,
    pub skipped: usize,
}

impl std::fmt::Display for MigrateStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "read={} inserted={} skipped={}",
            self.read, self.inserted, self.skipped
        )
    }
}

fn resolve_pg_url(cli: &Cli) -> String {
    if let Some(u) = &cli.pg_url {
        return u.clone();
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "postgres".into());
    format!("postgres://{user}@localhost:5432/daimon")
}

fn resolve_sqlite_dir(cli: &Cli) -> PathBuf {
    cli.sqlite_dir.clone().unwrap_or_else(|| PathBuf::from("./daimon-data"))
}

async fn resolve_tenant_id(
    pool: &deadpool_postgres::Pool,
    slug: &str,
) -> Result<Uuid> {
    let client = pool.get().await.context("get pg client")?;
    let row = client
        .query_one("SELECT id FROM public.tenants WHERE slug = $1", &[&slug])
        .await
        .with_context(|| format!("tenant lookup: {slug}"))?;
    Ok(row.get(0))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cli = Cli::parse();
    let url = resolve_pg_url(&cli);
    let sqlite_dir = resolve_sqlite_dir(&cli);
    let pool = daimon_db::build_pool(&url).context("build pg pool")?;
    let tenant_id = resolve_tenant_id(&pool, &cli.tenant_slug).await?;

    match cli.cmd {
        Cmd::Run { dry_run } => {
            let vault_stats = vault::migrate(&pool, &sqlite_dir.join("vault.db"), tenant_id, dry_run).await?;
            let inv_stats = inventory::migrate(&pool, &sqlite_dir.join("inventory.db"), tenant_id, dry_run).await?;
            let app_stats = app::migrate(&pool, &cli.app_db, tenant_id, dry_run).await?;
            let audit_stats = audit::migrate(&pool, &sqlite_dir.join("audit.db"), tenant_id, dry_run).await?;
            println!("== migrate complete ({}) ==", if dry_run { "dry-run" } else { "live" });
            println!("vault      {}", vault_stats);
            println!("inventory  {}", inv_stats);
            println!("app        {}", app_stats);
            println!("audit      {}", audit_stats);
        }
        Cmd::Vault { dry_run } => {
            let s = vault::migrate(&pool, &sqlite_dir.join("vault.db"), tenant_id, dry_run).await?;
            println!("vault {}", s);
        }
        Cmd::Inventory { dry_run } => {
            let s = inventory::migrate(&pool, &sqlite_dir.join("inventory.db"), tenant_id, dry_run).await?;
            println!("inventory {}", s);
        }
        Cmd::Audit { dry_run } => {
            let s = audit::migrate(&pool, &sqlite_dir.join("audit.db"), tenant_id, dry_run).await?;
            println!("audit {}", s);
        }
        Cmd::App { dry_run } => {
            let s = app::migrate(&pool, &cli.app_db, tenant_id, dry_run).await?;
            println!("app {}", s);
        }
        Cmd::Verify => {
            verify_all(&pool, &sqlite_dir, &cli.app_db, tenant_id).await?;
        }
    }

    Ok(())
}

async fn verify_all(
    pool: &deadpool_postgres::Pool,
    sqlite_dir: &PathBuf,
    app_db: &PathBuf,
    tenant_id: Uuid,
) -> Result<()> {
    let client = pool.get().await?;

    let row_counts = |path: &PathBuf, sql: &str| -> Result<i64> {
        let conn = rusqlite::Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        let n: i64 = conn.query_row(sql, [], |r| r.get(0))?;
        Ok(n)
    };

    let sqlite_vault = row_counts(&sqlite_dir.join("vault.db"), "SELECT COUNT(*) FROM credentials")?;
    let sqlite_inv = row_counts(&sqlite_dir.join("inventory.db"), "SELECT COUNT(*) FROM targets")?;
    let sqlite_audit = row_counts(&sqlite_dir.join("audit.db"), "SELECT COUNT(*) FROM audit_events")?;
    let sqlite_users = row_counts(app_db, "SELECT COUNT(*) FROM users")?;
    let sqlite_sess = row_counts(app_db, "SELECT COUNT(*) FROM sessions")?;
    let sqlite_clusters = row_counts(app_db, "SELECT COUNT(*) FROM clusters")?;
    let sqlite_config = row_counts(app_db, "SELECT COUNT(*) FROM config")?;
    let sqlite_prefs = row_counts(app_db, "SELECT COUNT(*) FROM user_preferences")?;

    async fn pg_count(
        client: &deadpool_postgres::Client,
        table: &str,
        tenant_id: Uuid,
    ) -> Result<i64> {
        let sql = format!("SELECT COUNT(*) FROM {} WHERE tenant_id = $1", table);
        let row = client.query_one(sql.as_str(), &[&tenant_id]).await?;
        Ok(row.get::<_, i64>(0))
    }

    let pg_vault = pg_count(&client, "vault.credentials", tenant_id).await?;
    let pg_inv = pg_count(&client, "inventory.targets", tenant_id).await?;
    let pg_audit = pg_count(&client, "audit.events", tenant_id).await?;
    let pg_users = pg_count(&client, "public.users", tenant_id).await?;
    let pg_clusters = pg_count(&client, "public.clusters", tenant_id).await?;

    // sessions + app_config + user_preferences are not tenant-scoped (yet)
    let pg_sess: i64 = client.query_one("SELECT COUNT(*) FROM public.sessions", &[]).await?.get(0);
    let pg_config: i64 = client.query_one("SELECT COUNT(*) FROM public.app_config", &[]).await?.get(0);
    let pg_prefs: i64 = client.query_one("SELECT COUNT(*) FROM public.user_preferences", &[]).await?.get(0);

    println!("{:<24} {:>8} {:>8} {:>6}", "table", "sqlite", "pg", "match");
    let report = |name: &str, a: i64, b: i64| {
        let ok = if a == b { "OK" } else { "DIFF" };
        println!("{:<24} {:>8} {:>8} {:>6}", name, a, b, ok);
    };
    report("vault.credentials", sqlite_vault, pg_vault);
    report("inventory.targets", sqlite_inv, pg_inv);
    report("audit.events", sqlite_audit, pg_audit);
    report("public.users", sqlite_users, pg_users);
    report("public.sessions", sqlite_sess, pg_sess);
    report("public.clusters", sqlite_clusters, pg_clusters);
    report("public.app_config", sqlite_config, pg_config);
    report("public.user_preferences", sqlite_prefs, pg_prefs);
    Ok(())
}
