//! KMS adapter for the vault master DEK (Phase 2c D4).
//!
//! `daimon-vault` holds a 32-byte master DEK in process memory and uses it
//! to seal per-row credential payloads. Phase 2c moves that DEK at rest to
//! a KMS-wrapped ciphertext blob so the raw key never lives on the same
//! disk as the database. Boot reads the wrapped blob, unwraps via KMS,
//! holds the plaintext DEK in `Zeroizing` memory only.
//!
//! `KmsClient` is the trait every provider implements. Four impls ship in
//! Phase 2c:
//!
//! - `LocalFile` — DEFAULT for homelab / dev. Passthrough: wrap and unwrap
//!   are identity. The DEK file IS the raw 32-byte key. Insecure in any
//!   adversarial environment; use only when the deployment is single-host
//!   and the DEK file is OS-permission-protected. Logs a startup WARN.
//! - `VaultTransit` — HashiCorp Vault Transit secrets engine. The
//!   recommended production default for self-hosted deployments. The Vault
//!   server holds the KEK; daimon's wrapped DEK is just a ciphertext blob.
//! - `AwsKms` — Phase 2c stub. Returns `KmsError::NotImplemented`. Lands
//!   when the first cloud-hosted customer arrives.
//! - `Pkcs11` — Phase 2c stub. Same; for on-prem HSMs (Thales Luna, nCipher
//!   nShield, YubiHSM) when the first banking deployment requires it.
//!
//! Per MASTERPLAN.md §4.2 and plans/2026-05-23-phase-2c-compliance-posture-plan.md D4.

mod aws_kms;
mod local_file;
mod pkcs11;
mod vault_transit;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroizing;

pub use aws_kms::AwsKms;
pub use local_file::LocalFileKms;
pub use pkcs11::Pkcs11Kms;
pub use vault_transit::VaultTransitKms;

/// Plaintext DEK — 32 bytes, ZeroizeOnDrop via `Zeroizing`.
pub type PlainDek = Zeroizing<Vec<u8>>;

/// Provider that can wrap and unwrap a Data Encryption Key (DEK) against a
/// Key Encryption Key (KEK) held inside the KMS.
#[async_trait]
pub trait KmsClient: Send + Sync {
    /// Unwrap a ciphertext blob into the plaintext DEK. The returned value
    /// is zeroized when dropped.
    async fn unwrap_dek(&self, wrapped: &[u8]) -> Result<PlainDek, KmsError>;

    /// Wrap a plaintext DEK into a ciphertext blob safe to persist on disk.
    /// Used by `daimon vault rotate-dek`.
    async fn wrap_dek(&self, plaintext: &[u8]) -> Result<Vec<u8>, KmsError>;

    /// Identifier used in audit metadata + structured logs.
    fn id(&self) -> &'static str;
}

#[derive(Debug, Error)]
pub enum KmsError {
    #[error("kms `{0}` not implemented yet — install the matching provider crate")]
    NotImplemented(&'static str),
    #[error("kms provider misconfigured: {0}")]
    Config(String),
    #[error("kms transport error: {0}")]
    Transport(String),
    #[error("kms wrap/unwrap failed: {0}")]
    Crypto(String),
    #[error("kms i/o: {0}")]
    Io(#[from] std::io::Error),
}

/// On-disk format for a KMS-wrapped master DEK. JSON to leave headroom for
/// per-tenant key rotation + multi-version envelopes later.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedDekEnvelope {
    /// Provider id (`local_file`, `vault_transit`, `aws_kms`, `pkcs11`).
    pub provider: String,
    /// Provider-defined key id / version. Empty for `local_file`.
    pub kek_ref: String,
    /// Schema version for this envelope. Bump when the envelope shape changes.
    pub version: u32,
    /// Base64-encoded wrapped DEK ciphertext (or raw key for `local_file`).
    pub ciphertext_b64: String,
}

impl WrappedDekEnvelope {
    pub fn new(provider: impl Into<String>, kek_ref: impl Into<String>, ciphertext: &[u8]) -> Self {
        use base64::Engine;
        Self {
            provider: provider.into(),
            kek_ref: kek_ref.into(),
            version: 1,
            ciphertext_b64: base64::engine::general_purpose::STANDARD.encode(ciphertext),
        }
    }

    pub fn ciphertext(&self) -> Result<Vec<u8>, KmsError> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(&self.ciphertext_b64)
            .map_err(|e| KmsError::Crypto(format!("base64: {e}")))
    }
}
