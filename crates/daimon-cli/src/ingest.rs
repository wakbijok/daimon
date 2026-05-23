//! `daimon-ingest` — operator CLI for ingesting a file into a tenant's long-term memory.
//!
//! Usage:
//!   daimon-ingest --tenant <id> --source <path> [--source-id <id>] [--kind <kind>] [--qdrant <url>]
//!
//! Reads the file, chunks via daimon-rag default config, embeds via fastembed (bge-small-en-v1.5
//! by default — downloads on first run), upserts into `tenant_<id>_long_term` Qdrant collection.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use daimon_memory::VectorStore;
use daimon_rag::{ChunkConfig, Document, Embedder, SparseEmbedder, ingest_document};

#[derive(Parser, Debug)]
#[command(name = "daimon-ingest", about = "Ingest a file into a tenant's long-term memory")]
struct Args {
    /// Tenant identifier — controls Qdrant collection scope. Resolved against
    /// public.tenants (slug column) for the Postgres canonical-content tier.
    #[arg(long, default_value = "default")]
    tenant: String,

    /// Postgres connection URL. Defaults to $DAIMON_PG_URL or
    /// postgres://$USER@localhost:5432/daimon.
    #[arg(long, env = "DAIMON_PG_URL")]
    pg_url: Option<String>,

    /// Path to the source file to ingest.
    #[arg(long)]
    source: PathBuf,

    /// Stable source identifier. Defaults to the source file's basename.
    #[arg(long)]
    source_id: Option<String>,

    /// Source kind label (e.g. "fiction", "doc", "runbook"). Stored in payload.
    #[arg(long, default_value = "doc")]
    kind: String,

    /// Qdrant gRPC URL.
    #[arg(long, default_value = "http://localhost:6334")]
    qdrant: String,

    /// Chunk size in tokens.
    #[arg(long, default_value_t = 512)]
    chunk_tokens: usize,

    /// Chunk overlap in tokens.
    #[arg(long, default_value_t = 64)]
    overlap_tokens: usize,
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

    let source_id = args
        .source_id
        .clone()
        .or_else(|| {
            args.source
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .context("could not derive source_id from --source path")?;

    let content = fs::read_to_string(&args.source)
        .with_context(|| format!("read {}", args.source.display()))?;

    let chunk_cfg = ChunkConfig {
        chunk_tokens: args.chunk_tokens,
        overlap_tokens: args.overlap_tokens,
    };

    eprintln!(
        "tenant={} source={} ({} bytes) kind={} qdrant={}",
        args.tenant,
        source_id,
        content.len(),
        args.kind,
        args.qdrant
    );

    eprintln!("connecting to qdrant...");
    let store = VectorStore::connect(&args.qdrant).context("qdrant connect")?;

    eprintln!("connecting to postgres...");
    let pg_url = args.pg_url.clone().unwrap_or_else(|| {
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".into());
        format!("postgres://{user}@localhost:5432/daimon")
    });
    let pool = daimon_db::build_pool(&pg_url).context("pg pool")?;

    eprintln!("resolving tenant `{}`...", args.tenant);
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

    eprintln!("loading dense embedder (first run downloads model, ~33MB)...");
    let embedder = Embedder::new_default().context("embedder init")?;
    eprintln!("dense embedder ready, dim={}", embedder.dim());

    eprintln!("loading sparse embedder (SPLADE++, first run downloads ~100MB)...");
    let sparse = SparseEmbedder::new_default().context("sparse embedder init")?;

    let doc = Document {
        source_id: source_id.clone(),
        source_kind: args.kind.clone(),
        content,
    };

    eprintln!("ingesting (dense + sparse)...");
    let stats = ingest_document(
        &pool,
        &store,
        &embedder,
        &sparse,
        tenant_uuid,
        &args.tenant,
        &doc,
        &chunk_cfg,
    )
    .await
    .context("ingest")?;

    println!(
        "ok: tenant={} source_id={} document_id={} chunks={} collection={} skipped={}",
        args.tenant,
        stats.source_id,
        stats.document_id,
        stats.chunks,
        stats.collection,
        stats.skipped_unchanged,
    );

    Ok(())
}
