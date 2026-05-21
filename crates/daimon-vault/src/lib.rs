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
//! Phase 2b storage: `SqliteVaultClient` — file-backed SQLite + per-row
//! XChaCha20-Poly1305 + 5-min TTL LRU cache. `MasterKey` reads from systemd
//! `LoadCredentialEncrypted=` at startup. CRUD surface (create / list /
//! reveal / update / rename / delete) consumed by the broker's admin proxy.
//!
//! See `docs/specs/2026-05-20-multi-agent-architecture-design.md`:
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
pub mod refspec;
#[cfg(feature = "for-broker")]
pub mod sealed;
#[cfg(feature = "for-broker")]
pub mod sqlite_client;

#[cfg(feature = "for-broker")]
pub use client::{StubVaultClient, VaultClient, VaultError};
#[cfg(feature = "for-broker")]
pub use credential::{Credential, CredentialKind};
#[cfg(feature = "for-broker")]
pub use master_key::{MasterKey, MasterKeyError};
#[cfg(feature = "for-broker")]
pub use refspec::{CredentialRef, RefParseError};
#[cfg(feature = "for-broker")]
pub use sealed::{SealError, SealedSession};
#[cfg(feature = "for-broker")]
pub use sqlite_client::{CredentialMetadata, SqliteVaultClient};

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
#[cfg(all(test, not(feature = "for-broker")))]
mod sqlite_client;
