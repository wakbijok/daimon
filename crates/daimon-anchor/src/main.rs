//! daimon-anchor — operator CLI for tamper-evidence anchoring and verification.
//!
//! Subcommands:
//!   snapshot     Write a manifest for the global chain head
//!   verify       Walk + verify the global chain
//!   list         List recent anchors
//!
//! Connection: $DAIMON_PG_URL (default postgres://$USER@localhost:5432/daimon).
//! Anchor mirror: $DAIMON_ANCHOR_DIR (default $DAIMON_DATA_DIR/anchors,
//! falling back to ./daimon-data/anchors).

mod canonical;
mod snapshot;
mod verify;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "daimon-anchor", about = "Audit hash-chain anchoring + verification")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,

    /// Postgres connection URL. Default $DAIMON_PG_URL or
    /// postgres://$USER@localhost:5432/daimon.
    #[arg(long, env = "DAIMON_PG_URL")]
    pg_url: Option<String>,

    /// Directory for the file-mirrored anchor manifests. Default
    /// $DAIMON_DATA_DIR/anchors or ./daimon-data/anchors.
    #[arg(long, env = "DAIMON_ANCHOR_DIR")]
    anchor_dir: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Snapshot the global chain head.
    Snapshot {
        /// Skip the file mirror; write only to audit.anchors.
        #[arg(long)]
        no_file: bool,
    },
    /// Verify the global audit chain.
    Verify,
    /// List recent anchors.
    List {
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
}

fn resolve_pg_url(cli: &Cli) -> String {
    if let Some(u) = &cli.pg_url {
        return u.clone();
    }
    if let Ok(u) = std::env::var("DAIMON_PG_URL") {
        return u;
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "postgres".into());
    format!("postgres://{user}@localhost:5432/daimon")
}

fn resolve_anchor_dir(cli: &Cli) -> PathBuf {
    if let Some(p) = &cli.anchor_dir {
        return p.clone();
    }
    if let Ok(p) = std::env::var("DAIMON_ANCHOR_DIR") {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("DAIMON_DATA_DIR") {
        return PathBuf::from(p).join("anchors");
    }
    PathBuf::from("./daimon-data/anchors")
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cli = Cli::parse();
    let url = resolve_pg_url(&cli);
    let dir = resolve_anchor_dir(&cli);
    let pool = daimon_db::build_pool(&url).context("build pg pool")?;

    match cli.cmd {
        Cmd::Snapshot { no_file } => {
            let anchor_dir = if no_file { None } else { Some(dir) };
            let instance_id = Uuid::new_v4();

            let s = snapshot::snapshot(&pool, instance_id, anchor_dir.as_ref()).await?;
            println!(
                "anchored as_of={} row_hash={} rows={} file={}",
                s.manifest.as_of_ts,
                s.manifest.row_hash_hex,
                s.manifest.row_count,
                s.written_to
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<none>".into())
            );
        }
        Cmd::Verify => {
            let report = verify::verify(&pool).await?;
            println!(
                "rows_checked={} breaks={}",
                report.rows_checked,
                report.breaks.len()
            );
            for b in &report.breaks {
                println!(
                    "  BREAK at ts={} id={} stored={} expected={}",
                    b.ts, b.event_id, b.stored_hex, b.expected_hex
                );
            }
            if !report.breaks.is_empty() {
                bail!("chain verification failed: {} break(s)", report.breaks.len());
            }
        }
        Cmd::List { limit } => {
            let client = pool.get().await?;
            let rows = client
                .query(
                    "SELECT as_of_ts, encode(row_hash, 'hex'), row_count
                     FROM audit.anchors
                     ORDER BY as_of_ts DESC
                     LIMIT $1",
                    &[&limit],
                )
                .await?;
            println!("anchors:");
            for r in rows {
                let ts: chrono::DateTime<chrono::Utc> = r.get(0);
                let h: String = r.get(1);
                let n: i64 = r.get(2);
                println!("  {} rows={} hash={}", ts, n, &h[..16]);
            }
        }
    }

    Ok(())
}
