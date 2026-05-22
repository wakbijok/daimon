//! `daimon-retrieve` — operator CLI for querying a tenant's long-term memory.
//!
//! Usage:
//!   daimon-retrieve --tenant <id> --query "..." [--top-k 5] [--qdrant <url>]
//!
//! Embeds the query via the same fastembed model used at ingest, runs vector search
//! against the tenant's `tenant_<id>_long_term` collection, prints top-K hits with
//! score + snippet.

use anyhow::{Context, Result};
use clap::Parser;
use daimon_memory::VectorStore;
use daimon_rag::{Embedder, retrieve};

#[derive(Parser, Debug)]
#[command(name = "daimon-retrieve", about = "Retrieve from a tenant's long-term memory")]
struct Args {
    /// Tenant identifier — controls Qdrant collection scope.
    #[arg(long)]
    tenant: String,

    /// Natural-language query.
    #[arg(long)]
    query: String,

    /// Number of results to return.
    #[arg(long, default_value_t = 5)]
    top_k: u64,

    /// Qdrant gRPC URL.
    #[arg(long, default_value = "http://localhost:6334")]
    qdrant: String,

    /// Maximum characters of each hit's text to print. 0 = no truncation.
    #[arg(long, default_value_t = 280)]
    snippet_len: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,daimon_rag=info,daimon_memory=info".into()),
        )
        .init();

    let args = Args::parse();

    eprintln!("connecting to qdrant {} ...", args.qdrant);
    let store = VectorStore::connect(&args.qdrant).context("qdrant connect")?;

    eprintln!("loading embedder...");
    let embedder = Embedder::new_default().context("embedder init")?;

    eprintln!("retrieving top {} for: {}", args.top_k, args.query);
    let hits = retrieve(&store, &embedder, &args.tenant, &args.query, args.top_k)
        .await
        .context("retrieve")?;

    if hits.is_empty() {
        eprintln!("(no hits — tenant collection empty or unknown)");
        return Ok(());
    }

    for (i, h) in hits.iter().enumerate() {
        let text = h
            .payload
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let source_id = h
            .payload
            .get("source_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let snippet = if args.snippet_len > 0 && text.len() > args.snippet_len {
            format!("{}…", &text[..args.snippet_len])
        } else {
            text.to_string()
        };
        println!(
            "#{} score={:.4} id={} source={}\n    {}",
            i + 1,
            h.score,
            h.id,
            source_id,
            snippet
        );
    }

    Ok(())
}
