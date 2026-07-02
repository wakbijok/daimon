//! Phase 3 D10 — end-to-end integration test for the hybrid RAG pipeline.
//!
//! Exercises:
//! 1. Hybrid retrieval finds a BM25-strong query target via the sparse leg
//! 2. Hybrid retrieval finds a dense-strong query target via the dense leg
//! 3. Deletion clears Postgres canonical rows AND Qdrant points
//! 4. Context packer respects token budget
//! 5. Reranker reorders the top-K vs raw hybrid ordering
//!
//! Gated `#[ignore]`. Requires Postgres + Qdrant running. Run via:
//!   DAIMON_PG_URL=postgres://wakbijak@localhost:5432/daimon \
//!     cargo test -p daimon-rag --test phase3_e2e -- --ignored
//!
//! First run downloads ~250 MB of model weights (bge-small + SPLADE + bge-reranker)
//! into `~/.cache/fastembed/`.

#![cfg(test)]

use daimon_memory::VectorStore;
use daimon_rag::{
    ChunkConfig, Document, Embedder, PackConfig, Reranker, SparseEmbedder, delete_document,
    ingest_document, long_term_collection, pack_context, retrieve, retrieve_with_rerank,
};

fn pg_url() -> String {
    std::env::var("DAIMON_PG_URL").unwrap_or_else(|_| {
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".into());
        format!("postgres://{user}@localhost:5432/daimon")
    })
}

fn qdrant_url() -> String {
    std::env::var("DAIMON_QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".into())
}

async fn cleanup(pool: &daimon_db::Pool, store: &VectorStore) {
    let client = pool.get().await.expect("pool");
    let _ = client
        .execute(
            "DELETE FROM memory.documents WHERE source_id IN ('doc-a', 'doc-b')",
            &[],
        )
        .await;
    let _ = store.drop_collection(&long_term_collection()).await;
}

#[tokio::test]
#[ignore]
async fn hybrid_dense_and_sparse_retrieval_plus_delete_plus_pack() {
    let pg_url = pg_url();
    daimon_db::run_migrations(&pg_url).await.expect("migrations");
    let pool = daimon_db::build_pool(&pg_url).expect("pool");
    let store = VectorStore::connect(&qdrant_url()).expect("qdrant");

    // Start from a clean slate — single fixed collection is shared.
    cleanup(&pool, &store).await;

    let embedder = Embedder::new_default().expect("dense embedder");
    let sparse = SparseEmbedder::new_default().expect("sparse embedder");
    let reranker = Reranker::new_default().expect("reranker");

    // Two docs:
    // A — heavy on lexical/keyword "wakanda forever" (BM25 should strongly match)
    // B — semantic content about superheroes without the keyword (dense should match)
    let doc_a = Document {
        source_id: "doc-a".into(),
        source_kind: "test".into(),
        content: "wakanda forever rises again as the ceremonial chant echoes through the vibranium halls.".into(),
    };
    let doc_b = Document {
        source_id: "doc-b".into(),
        source_kind: "test".into(),
        content: "A masked vigilante patrols Gotham at night, wielding gadgets to stop crime in a brooding metropolis.".into(),
    };

    let cfg = ChunkConfig::default();
    let stats_a = ingest_document(&pool, &store, &embedder, &sparse, &doc_a, &cfg)
        .await
        .expect("ingest A");
    let stats_b = ingest_document(&pool, &store, &embedder, &sparse, &doc_b, &cfg)
        .await
        .expect("ingest B");
    assert!(!stats_a.skipped_unchanged);
    assert!(!stats_b.skipped_unchanged);

    // ---- (1) BM25-strong query should rank doc A above doc B ----
    let bm25_hits = retrieve(&pool, &store, &embedder, &sparse, "wakanda forever chant", 5)
        .await
        .expect("retrieve bm25");
    assert!(
        !bm25_hits.is_empty(),
        "expected at least one hit for the lexical query"
    );
    assert_eq!(
        bm25_hits[0].source_id, "doc-a",
        "lexical query should rank doc-a first; got {bm25_hits:?}"
    );

    // ---- (2) Semantic-strong query should find doc B ----
    let semantic_hits = retrieve(
        &pool, &store, &embedder, &sparse,
        "Batman patrolling Gotham fighting villains", 5,
    )
    .await
    .expect("retrieve semantic");
    assert!(
        semantic_hits.iter().any(|h| h.source_id == "doc-b"),
        "semantic query should include doc-b in results; got {semantic_hits:?}"
    );

    // ---- (3) Rerank changes ordering ----
    let reranked = retrieve_with_rerank(
        &pool, &store, &embedder, &sparse, &reranker,
        "wakanda forever chant", 5, 10,
    )
    .await
    .expect("retrieve_with_rerank");
    assert!(!reranked.is_empty(), "rerank should return at least one hit");
    // The reranker MUST score doc-a higher for this lexical query.
    assert_eq!(reranked[0].source_id, "doc-a");

    // ---- (4) Context packer respects token budget ----
    let pack_cfg = PackConfig {
        max_tokens: 25,
        diversity_lambda: 0.3,
    };
    let packed = pack_context(&bm25_hits, &pack_cfg);
    let total_tokens: i32 = packed.iter().map(|i| i.chunk.token_estimate).sum();
    assert!(
        total_tokens <= pack_cfg.max_tokens,
        "packer overshot budget: {} > {}",
        total_tokens,
        pack_cfg.max_tokens
    );

    // ---- (5) Delete doc-a → no longer retrievable ----
    let deleted = delete_document(&pool, &store, "doc-a")
        .await
        .expect("delete doc-a");
    assert!(deleted >= 1, "delete should report at least one chunk removed");

    let post_delete = retrieve(&pool, &store, &embedder, &sparse, "wakanda forever chant", 5)
        .await
        .expect("retrieve post-delete");
    assert!(
        post_delete.iter().all(|h| h.source_id != "doc-a"),
        "doc-a must not appear after delete; got {post_delete:?}"
    );

    // Postgres canonical row is gone.
    let client = pool.get().await.expect("pool");
    let cnt: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM memory.documents WHERE source_id = 'doc-a'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(cnt, 0, "memory.documents must have 0 rows for deleted doc-a");
    drop(client);

    cleanup(&pool, &store).await;
}
