//! `daimon-retrieve` — operator CLI for querying a tenant's long-term memory.
//!
//! Usage:
//!   daimon-retrieve --tenant <slug> --query "..." [--top-k 5] [--qdrant <url>]
//!
//! Phase 3 D5: chunk text comes from Postgres canonical tier (memory.document_chunks)
//! via JOIN against Qdrant's returned point ids. Print top-K hits with score + snippet.

use anyhow::{Context, Result};
use clap::Parser;
use daimon_memory::VectorStore;
use daimon_rag::{Embedder, SparseEmbedder, retrieve};

#[derive(Parser, Debug)]
#[command(name = "daimon-retrieve", about = "Retrieve from a tenant's long-term memory")]
struct Args {
    /// Tenant slug (must exist in public.tenants).
    #[arg(long, default_value = "default")]
    tenant: String,

    /// Postgres connection URL. Defaults to $DAIMON_PG_URL or
    /// postgres://$USER@localhost:5432/daimon.
    #[arg(long, env = "DAIMON_PG_URL")]
    pg_url: Option<String>,

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

    eprintln!("connecting to postgres...");
    let pg_url = args.pg_url.clone().unwrap_or_else(|| {
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".into());
        format!("postgres://{user}@localhost:5432/daimon")
    });
    let pool = daimon_db::build_pool(&pg_url).context("pg pool")?;

    let client = pool.get().await.context("pg client")?;
    let row = client
        .query_one(
            "SELECT id FROM public.tenants WHERE slug = $1",
            &[&args.tenant],
        )
        .await
        .with_context(|| format!("tenant `{}` not found", args.tenant))?;
    let tenant_uuid: uuid::Uuid = row.get(0);
    drop(client);

    eprintln!("loading dense embedder...");
    let embedder = Embedder::new_default().context("embedder init")?;
    eprintln!("loading sparse embedder...");
    let sparse = SparseEmbedder::new_default().context("sparse init")?;

    eprintln!("retrieving top {} (hybrid dense+sparse) for: {}", args.top_k, args.query);
    let hits = retrieve(
        &pool,
        &store,
        &embedder,
        &sparse,
        tenant_uuid,
        &args.tenant,
        &args.query,
        args.top_k,
    )
    .await
    .context("retrieve")?;

    if hits.is_empty() {
        eprintln!("(no hits — tenant collection empty or unknown)");
        return Ok(());
    }

    for (i, h) in hits.iter().enumerate() {
        let snippet = if args.snippet_len > 0 && h.content.len() > args.snippet_len {
            format!("{}…", &h.content[..args.snippet_len])
        } else {
            h.content.clone()
        };
        println!(
            "#{} score={:.4} chunk_id={} source={} (kind={})\n    {}",
            i + 1,
            h.score,
            h.chunk_id,
            h.source_id,
            h.source_kind,
            snippet
        );
    }

    Ok(())
}
