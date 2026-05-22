//! Full RAG roundtrip integration test. Requires:
//! - Local Qdrant running on localhost:6334 gRPC (`just qdrant-up`)
//! - bge-small model cached or downloadable (first run pulls ~33MB)
//!
//! Run: `cargo test -p daimon-rag --test end_to_end -- --nocapture`

use daimon_memory::VectorStore;
use daimon_rag::{ChunkConfig, Document, Embedder, ingest_document, retrieve};
use rand::Rng;

const QDRANT_URL: &str = "http://localhost:6334";

fn rand_tenant() -> String {
    let mut rng = rand::thread_rng();
    let n: u32 = rng.r#gen();
    format!("test_{:x}", n)
}

fn lotr_paragraph() -> &'static str {
    "In a hole in the ground there lived a hobbit. Not a nasty, dirty, wet hole, \
     filled with the ends of worms and an oozy smell, nor yet a dry, bare, sandy hole \
     with nothing in it to sit down on or to eat: it was a hobbit-hole, and that means comfort. \
     It had a perfectly round door like a porthole, painted green, with a shiny yellow brass knob \
     in the exact middle. The door opened on to a tube-shaped hall like a tunnel: a very comfortable \
     tunnel without smoke, with panelled walls, and floors tiled and carpeted, provided with polished \
     chairs, and lots and lots of pegs for hats and coats — the hobbit was fond of visitors."
}

fn quantum_paragraph() -> &'static str {
    "Quantum entanglement is a phenomenon where pairs of particles are generated in such a way \
     that the quantum state of each particle cannot be described independently of the state of \
     the others, even when separated by large distances. Measurement of one particle's spin \
     instantaneously determines the other's correlated spin, regardless of separation. Bell's \
     theorem establishes that no theory of local hidden variables can ever reproduce all of the \
     predictions of quantum mechanics."
}

#[tokio::test]
async fn ingest_then_retrieve_finds_relevant_chunk() {
    let store = VectorStore::connect(QDRANT_URL)
        .expect("connect to qdrant — is `just qdrant-up` running?");
    let embedder = Embedder::new_default().expect("init embedder");

    let tenant = rand_tenant();

    let doc_a = Document {
        source_id: "lotr-hobbit-opening".into(),
        source_kind: "fiction".into(),
        content: lotr_paragraph().into(),
    };
    let doc_b = Document {
        source_id: "quantum-physics-bell".into(),
        source_kind: "physics".into(),
        content: quantum_paragraph().into(),
    };

    let stats_a = ingest_document(&store, &embedder, &tenant, &doc_a, &ChunkConfig::default())
        .await
        .expect("ingest doc_a");
    let stats_b = ingest_document(&store, &embedder, &tenant, &doc_b, &ChunkConfig::default())
        .await
        .expect("ingest doc_b");

    eprintln!(
        "ingested: doc_a {} chunks → {}, doc_b {} chunks → {}",
        stats_a.chunks, stats_a.collection, stats_b.chunks, stats_b.collection
    );

    let query = "Where does Bilbo Baggins live?";
    let hits = retrieve(&store, &embedder, &tenant, query, 3)
        .await
        .expect("retrieve");

    assert!(!hits.is_empty(), "expected at least one hit");
    eprintln!("top hit score = {:.4}", hits[0].score);
    eprintln!(
        "top hit payload source_id = {:?}",
        hits[0].payload.get("source_id")
    );

    let top_source = hits[0]
        .payload
        .get("source_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_eq!(
        top_source, "lotr-hobbit-opening",
        "top hit should come from the LOTR doc, got source_id={}",
        top_source
    );
    assert!(
        hits[0].score > 0.4,
        "top hit cosine {:.4} suspiciously low",
        hits[0].score
    );

    // Cleanup
    let collection = daimon_rag::long_term_collection(&tenant);
    store.drop_collection(&collection).await.ok();
}

#[tokio::test]
async fn tenant_isolation_holds() {
    let store = VectorStore::connect(QDRANT_URL).expect("connect");
    let embedder = Embedder::new_default().expect("init embedder");

    let tenant_a = rand_tenant();
    let tenant_b = rand_tenant();

    let doc = Document {
        source_id: "test-doc".into(),
        source_kind: "test".into(),
        content: lotr_paragraph().into(),
    };
    ingest_document(&store, &embedder, &tenant_a, &doc, &ChunkConfig::default())
        .await
        .expect("ingest into tenant_a");

    // Tenant B's collection doesn't exist; retrieve should ensure-and-search and return empty.
    let hits_b = retrieve(&store, &embedder, &tenant_b, "hobbit", 5)
        .await
        .expect("retrieve from tenant_b (empty collection ok)");
    assert!(
        hits_b.is_empty(),
        "tenant_b should see zero hits, got {}",
        hits_b.len()
    );

    // Tenant A sees its own content.
    let hits_a = retrieve(&store, &embedder, &tenant_a, "hobbit", 5).await.expect("retrieve from tenant_a");
    assert!(!hits_a.is_empty(), "tenant_a should see its own ingested content");

    // Cleanup
    store
        .drop_collection(&daimon_rag::long_term_collection(&tenant_a))
        .await
        .ok();
    store
        .drop_collection(&daimon_rag::long_term_collection(&tenant_b))
        .await
        .ok();
}
