//! daimon-memory — three-tier memory for daimon agents.
//!
//! Tiers:
//! - **Conversation** — append-only timeline per session, recent-window access
//! - **Working** — per-agent scratchpad K/V with TTL
//! - **Long-term** — vector-backed semantic store (Qdrant) + canonical payload (Postgres in prod, SQLite in dev)
//!
//! Phase 3 lands the long-term tier first (Qdrant via [`VectorStore`]). Conversation + working tiers
//! are scaffolded with Postgres-backed impls; both move to Redis in Phase 4.

pub mod error;
pub mod vector;

pub use error::{Error, Result};
pub use vector::{Hit, Point, VectorStore};
