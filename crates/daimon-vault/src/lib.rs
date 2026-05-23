//! In-tree credential vault for dAImon. **INTERNAL — broker-only (D21).**
//!
//! Worker crates MUST NOT add `daimon-vault` to their `Cargo.toml`. Only
//! `daimon-broker` depends on this crate and enables the `for-broker` feature.
//! Without that feature, the crate exports nothing.
//!
//! Phase 2a foundation: `CredentialRef` URL parsing, typed `Credential` enum
//! with secret redaction, `SealedSession` for XChaCha20-Poly1305 sealing,
//! `VaultClient` trait, `StubVaultClient` for tests.
//!
//! Phase 2b storage: SQLite-backed `SqliteVaultClient` (D22). REMOVED in
//! Phase 2c D3b — production uses `PostgresVaultClient` against the
//! relational tier owned by `daimon-db`.
//!
//! Phase 2c storage: `PostgresVaultClient` — pool-backed, per-row sealed,
//! tenant-scoped. 5-min LRU resolves cache. Master DEK from systemd
//! `LoadCredentialEncrypted=` (D22). KMS wrapping arrives in D4.
//!
//! See `daimon-docs/specs/2026-05-20-multi-agent-architecture-design.md`:
//! - D19 — broker pattern (workers never see credentials)
//! - D21 — worker dependency restriction (this crate is broker-only)
//! - D22 — in-tree vault (supersedes D3 + D4)

#[cfg(feature = "for-broker")]
pub mod client;
#[cfg(feature = "for-broker")]
pub mod credential;
#[cfg(feature = "for-broker")]
pub mod master_key;
#[cfg(feature = "for-broker")]
pub mod postgres_client;
#[cfg(feature = "for-broker")]
pub mod refspec;
#[cfg(feature = "for-broker")]
pub mod sealed;

#[cfg(feature = "for-broker")]
pub use client::{StubVaultClient, VaultClient, VaultError};
#[cfg(feature = "for-broker")]
pub use credential::{Credential, CredentialKind};
#[cfg(feature = "for-broker")]
pub use master_key::{MasterKey, MasterKeyError};
#[cfg(feature = "for-broker")]
pub use postgres_client::{CredentialMetadata, PostgresVaultClient};
#[cfg(feature = "for-broker")]
pub use refspec::{CredentialRef, RefParseError};
#[cfg(feature = "for-broker")]
pub use sealed::{SealError, SealedSession};

// Internal modules made available for in-crate unit tests even when the
// `for-broker` feature isn't enabled.
#[cfg(all(test, not(feature = "for-broker")))]
mod client;
#[cfg(all(test, not(feature = "for-broker")))]
mod credential;
#[cfg(all(test, not(feature = "for-broker")))]
mod master_key;
#[cfg(all(test, not(feature = "for-broker")))]
mod refspec;
#[cfg(all(test, not(feature = "for-broker")))]
mod sealed;
