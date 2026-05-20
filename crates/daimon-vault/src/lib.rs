//! Vaultwarden-backed credential resolution. **INTERNAL — broker-only (D21).**
//!
//! Worker crates MUST NOT add `daimon-vault` to their `Cargo.toml`. Only
//! `daimon-broker` depends on this crate and enables the `for-broker` feature.
//! Without that feature, the crate exports nothing.
//!
//! Phase 2 ships: `CredentialRef` URL parsing, typed `Credential` enum with
//! secret redaction, `SealedSession` for chacha20poly1305-protected
//! Vaultwarden session token persistence, and the `VaultClient` trait.
//!
//! Actual Vaultwarden REST integration lands in the Phase 2 continuation —
//! the foundation here is what `daimon-broker` builds against.
//!
//! See `docs/specs/2026-05-20-multi-agent-architecture-design.md` D3 (vault
//! choice), D4 (sealed-box bootstrap UX), D19 (broker pattern), D21
//! (dependency restriction).

#[cfg(feature = "for-broker")]
pub mod client;
#[cfg(feature = "for-broker")]
pub mod credential;
#[cfg(feature = "for-broker")]
pub mod refspec;
#[cfg(feature = "for-broker")]
pub mod sealed;

#[cfg(feature = "for-broker")]
pub use client::{StubVaultClient, VaultClient, VaultError};
#[cfg(feature = "for-broker")]
pub use credential::{Credential, CredentialKind};
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
mod refspec;
#[cfg(all(test, not(feature = "for-broker")))]
mod sealed;
