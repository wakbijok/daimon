//! AWS KMS provider — Phase 2c stub.
//!
//! Lands when the first cloud-hosted customer arrives. The impl will use
//! `aws-sdk-kms`'s `encrypt`/`decrypt` operations against a customer-owned
//! KMS key id. IAM auth via the standard SDK credential chain (env, IMDS,
//! AWS SSO, etc.).
//!
//! Until then, attempting to construct `AwsKms` yields
//! `KmsError::NotImplemented`. The trait surface is locked so callers can
//! write provider-agnostic code today.

use async_trait::async_trait;

use crate::{KmsClient, KmsError, PlainDek};

pub struct AwsKms {
    _key_id: String,
}

impl AwsKms {
    pub fn new(key_id: impl Into<String>) -> Result<Self, KmsError> {
        let _ = key_id;
        Err(KmsError::NotImplemented("aws_kms"))
    }
}

#[async_trait]
impl KmsClient for AwsKms {
    async fn unwrap_dek(&self, _wrapped: &[u8]) -> Result<PlainDek, KmsError> {
        Err(KmsError::NotImplemented("aws_kms"))
    }

    async fn wrap_dek(&self, _plaintext: &[u8]) -> Result<Vec<u8>, KmsError> {
        Err(KmsError::NotImplemented("aws_kms"))
    }

    fn id(&self) -> &'static str {
        "aws_kms"
    }
}
