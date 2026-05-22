//! `daimon-migrate` — apply pending Postgres migrations.
//!
//! Usage:
//!   DAIMON_PG_URL=postgres://... daimon-migrate

use anyhow::{Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,refinery=info,tokio_postgres=warn".into()),
        )
        .init();

    let url = std::env::var("DAIMON_PG_URL")
        .context("DAIMON_PG_URL not set — see `just pg-url`")?;

    eprintln!("connecting to {} ...", redact(&url));
    eprintln!("applying migrations...");
    daimon_db::run_migrations(&url)
        .await
        .context("run_migrations")?;
    eprintln!("ok");
    Ok(())
}

/// Redact password from a postgres URL for logging.
fn redact(url: &str) -> String {
    if let Some((scheme_and_auth, rest)) = url.split_once('@') {
        if let Some((prefix, _pw)) = scheme_and_auth.rsplit_once(':') {
            // Don't redact if the ':' is the scheme separator like "postgres:" with no user.
            if !prefix.ends_with("postgres") && !prefix.ends_with("postgresql") {
                return format!("{}:***@{}", prefix, rest);
            }
        }
    }
    url.to_string()
}
