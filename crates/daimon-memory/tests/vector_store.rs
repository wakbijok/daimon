//! Integration test for the Qdrant-backed [`VectorStore`].
//!
//! Requires a running Qdrant at localhost:6334 (gRPC). Bring one up with `just qdrant-up`.
//! The test creates a uniquely-named collection, exercises ensure/upsert/search, then drops it.
//!
//! Run with: `cargo test -p daimon-memory --test vector_store -- --nocapture`

use daimon_memory::{Point, VectorStore};
use rand::Rng;
use serde_json::json;

const QDRANT_URL: &str = "http://localhost:6334";
const DIM: u64 = 384;

fn rand_vec(dim: u64) -> Vec<f32> {
    let mut rng = rand::thread_rng();
    (0..dim).map(|_| rng.r#gen::<f32>()).collect()
}

fn rand_suffix() -> String {
    let mut rng = rand::thread_rng();
    let n: u32 = rng.r#gen();
    format!("{:x}", n)
}

#[tokio::test]
async fn smoke_ensure_upsert_search_drop() {
    let store = VectorStore::connect(QDRANT_URL)
        .expect("connect to qdrant at localhost:6334 — is `just qdrant-up` running?");

    let collection = format!("smoke_test_{}", rand_suffix());

    store
        .ensure_collection(&collection, DIM)
        .await
        .expect("ensure_collection");

    let vectors: Vec<Vec<f32>> = (0..5).map(|_| rand_vec(DIM)).collect();
    let points: Vec<Point> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| Point {
            id: (i + 1) as u64,
            vector: v.clone(),
            payload: json!({ "text": format!("doc {}", i + 1) }),
        })
        .collect();

    store
        .upsert(&collection, points)
        .await
        .expect("upsert");

    let query = vectors[2].clone();
    let hits = store
        .search(&collection, query, 3)
        .await
        .expect("search");

    assert_eq!(hits.len(), 3, "expected 3 hits, got {}", hits.len());
    assert_eq!(
        hits[0].id, 3,
        "top hit should be the queried vector (id 3), got id {}",
        hits[0].id
    );
    assert!(
        hits[0].score > 0.999,
        "self-similarity should be ~1.0 with cosine, got {}",
        hits[0].score
    );
    assert_eq!(
        hits[0].payload.get("text").and_then(|v| v.as_str()),
        Some("doc 3"),
        "payload should round-trip"
    );

    store
        .drop_collection(&collection)
        .await
        .expect("drop_collection");
}

#[tokio::test]
async fn ensure_collection_is_idempotent() {
    let store = VectorStore::connect(QDRANT_URL).expect("connect");
    let collection = format!("idempotent_test_{}", rand_suffix());

    for _ in 0..3 {
        store
            .ensure_collection(&collection, DIM)
            .await
            .expect("ensure_collection on repeat");
    }

    store.drop_collection(&collection).await.ok();
}
