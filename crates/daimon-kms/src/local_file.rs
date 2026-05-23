//! Passthrough provider — the DEK file IS the raw key.
//!
//! Use only when:
//! - the deployment is single-host
//! - the DEK file lives on an OS-permission-protected path (systemd
//!   `LoadCredentialEncrypted=`, mode 0400 root-owned, etc.)
//! - no compliance requirement mandates KMS / HSM separation
//!
//! Production banking / regulated deployments MUST use `VaultTransitKms`,
//! `AwsKms`, or `Pkcs11Kms`. A loud startup WARN is emitted when
//! `LocalFileKms` is selected.

use async_trait::async_trait;
use tracing::warn;
use zeroize::Zeroizing;

use crate::{KmsClient, KmsError, PlainDek};

#[derive(Debug, Clone, Copy, Default)]
pub struct LocalFileKms;

impl LocalFileKms {
    pub fn new() -> Self {
        warn!(
            "KMS provider = local_file — DEK is stored raw on disk. \
             Production deployments must use a real KMS (Vault Transit / AWS KMS / PKCS#11)."
        );
        Self
    }
}

#[async_trait]
impl KmsClient for LocalFileKms {
    async fn unwrap_dek(&self, wrapped: &[u8]) -> Result<PlainDek, KmsError> {
        Ok(Zeroizing::new(wrapped.to_vec()))
    }

    async fn wrap_dek(&self, plaintext: &[u8]) -> Result<Vec<u8>, KmsError> {
        Ok(plaintext.to_vec())
    }

    fn id(&self) -> &'static str {
        "local_file"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn passthrough_roundtrip() {
        let kms = LocalFileKms::new();
        let dek = vec![0x42u8; 32];
        let wrapped = kms.wrap_dek(&dek).await.unwrap();
        let unwrapped = kms.unwrap_dek(&wrapped).await.unwrap();
        assert_eq!(&dek, &**unwrapped);
    }
}
