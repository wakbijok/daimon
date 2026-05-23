//! PKCS#11 provider — Phase 2c stub.
//!
//! For on-prem HSMs (Thales Luna, nCipher nShield, YubiHSM). The impl will
//! use the `cryptoki` crate to talk to the configured PKCS#11 module, do
//! `C_EncryptInit`/`C_Encrypt` against the KEK label, and return the
//! resulting ciphertext. Lands when the first banking customer mandates
//! HSM-backed KEKs.

use async_trait::async_trait;

use crate::{KmsClient, KmsError, PlainDek};

pub struct Pkcs11Kms {
    _module_path: String,
    _slot_id: u32,
    _key_label: String,
}

impl Pkcs11Kms {
    pub fn new(
        module_path: impl Into<String>,
        slot_id: u32,
        key_label: impl Into<String>,
    ) -> Result<Self, KmsError> {
        let _ = (module_path.into(), slot_id, key_label.into());
        Err(KmsError::NotImplemented("pkcs11"))
    }
}

#[async_trait]
impl KmsClient for Pkcs11Kms {
    async fn unwrap_dek(&self, _wrapped: &[u8]) -> Result<PlainDek, KmsError> {
        Err(KmsError::NotImplemented("pkcs11"))
    }

    async fn wrap_dek(&self, _plaintext: &[u8]) -> Result<Vec<u8>, KmsError> {
        Err(KmsError::NotImplemented("pkcs11"))
    }

    fn id(&self) -> &'static str {
        "pkcs11"
    }
}
