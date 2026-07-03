//! Build script for daimon-db.
//!
//! `refinery::embed_migrations!("./migrations")` reads the migrations directory
//! at macro-expansion time and `include_str!`s the files it finds THEN. Cargo
//! tracks those specific files, but it does NOT know to re-run the macro when a
//! BRAND-NEW `.sql` file is added — so the embedded migration set can silently
//! go stale in a build cache that wasn't touched since the file appeared. If
//! `daimon-migrate` (one cache) then applies the new migration to the DB while
//! the app binary (a different target cache, e.g. cargo-leptos's
//! `target/<triple>/`) still has the stale embed, boot trips refinery's
//! "migration VNN is missing from the filesystem" divergence check.
//!
//! Emitting `rerun-if-changed` on the whole directory forces this crate (and
//! thus the embedded set) to rebuild whenever ANY migration file is added,
//! removed, or edited — closing that gap permanently.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
