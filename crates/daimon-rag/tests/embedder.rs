//! Integration test for [`daimon_rag::Embedder`]. Downloads the default model on first run
//! (~33MB to `~/.cache/fastembed/`). Subsequent runs are cached.

use daimon_rag::{Embedder, cosine};

#[test]
#[ignore = "downloads the fastembed model (~33MB); run explicitly with --ignored"]
fn embed_and_compare_semantics() {
    let embedder = Embedder::new_default().expect("init default embedder");
    assert_eq!(embedder.dim(), 384, "BGESmallENV15 dim is 384");

    let texts = [
        "The cat sat on the mat",
        "A feline rested on the rug",
        "Quantum entanglement violates local realism",
    ];

    let vecs = embedder.embed(&texts).expect("embed");
    assert_eq!(vecs.len(), 3, "got {} vectors", vecs.len());
    assert!(vecs.iter().all(|v| v.len() == 384), "all vecs should be 384d");

    let sim_related = cosine(&vecs[0], &vecs[1]);
    let sim_unrelated = cosine(&vecs[0], &vecs[2]);

    eprintln!("cosine(cat-on-mat, feline-on-rug) = {:.4}", sim_related);
    eprintln!("cosine(cat-on-mat, quantum-physics) = {:.4}", sim_unrelated);

    assert!(
        sim_related > sim_unrelated,
        "semantically related pair (cos={:.4}) should outscore unrelated pair (cos={:.4})",
        sim_related,
        sim_unrelated
    );

    // Sanity floor — sim_related should be a meaningful positive value for any decent embedder.
    assert!(
        sim_related > 0.5,
        "related pair cosine {:.4} suspiciously low — embedder model broken?",
        sim_related
    );
}
