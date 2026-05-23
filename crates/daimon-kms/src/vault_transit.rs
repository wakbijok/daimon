//! HashiCorp Vault Transit secrets engine provider.
//!
//! Vault Transit holds the KEK and exposes wrap/unwrap as `encrypt` and
//! `decrypt` HTTP operations. We POST a plaintext DEK to
//! `<vault_url>/v1/<mount>/encrypt/<key_name>` and store the returned
//! `ciphertext` as the wrapped envelope payload. On unwrap, the reverse.
//!
//! Required config:
//! - `vault_url` — e.g. `https://vault.internal:8200`
//! - `mount` — Transit mount path, default `transit`
//! - `key_name` — Transit key name (must be configured `derived=false`)
//! - `token` — Vault token with `update` perms on `<mount>/{encrypt,decrypt}/<key_name>`
//!
//! Token refresh / rotation is the deployer's responsibility. The client
//! does not auto-renew; for short-lived tokens, wrap `daimon-vault` in a
//! supervisor that reloads on a renew schedule.

use async_trait::async_trait;
use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::instrument;
use zeroize::Zeroizing;

use crate::{KmsClient, KmsError, PlainDek};

pub struct VaultTransitKms {
    http: Client,
    vault_url: String,
    mount: String,
    key_name: String,
    token: String,
}

#[derive(Serialize)]
struct EncryptReq {
    plaintext: String,
}

#[derive(Serialize)]
struct DecryptReq {
    ciphertext: String,
}

#[derive(Deserialize)]
struct WrappedResp {
    data: WrappedData,
}

#[derive(Deserialize)]
struct WrappedData {
    ciphertext: Option<String>,
    plaintext: Option<String>,
}

impl VaultTransitKms {
    pub fn new(
        vault_url: impl Into<String>,
        mount: impl Into<String>,
        key_name: impl Into<String>,
        token: impl Into<String>,
    ) -> Result<Self, KmsError> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| KmsError::Config(format!("http builder: {e}")))?;
        Ok(Self {
            http,
            vault_url: vault_url.into(),
            mount: mount.into(),
            key_name: key_name.into(),
            token: token.into(),
        })
    }

    fn encrypt_url(&self) -> String {
        format!(
            "{}/v1/{}/encrypt/{}",
            self.vault_url.trim_end_matches('/'),
            self.mount,
            self.key_name
        )
    }

    fn decrypt_url(&self) -> String {
        format!(
            "{}/v1/{}/decrypt/{}",
            self.vault_url.trim_end_matches('/'),
            self.mount,
            self.key_name
        )
    }
}

#[async_trait]
impl KmsClient for VaultTransitKms {
    #[instrument(skip(self, wrapped), level = "debug")]
    async fn unwrap_dek(&self, wrapped: &[u8]) -> Result<PlainDek, KmsError> {
        let ciphertext = std::str::from_utf8(wrapped)
            .map_err(|e| KmsError::Crypto(format!("vault ciphertext not utf-8: {e}")))?
            .trim()
            .to_string();
        let resp = self
            .http
            .post(self.decrypt_url())
            .header("X-Vault-Token", &self.token)
            .json(&DecryptReq { ciphertext })
            .send()
            .await
            .map_err(|e| KmsError::Transport(format!("decrypt: {e}")))?;
        if !resp.status().is_success() {
            return Err(KmsError::Crypto(format!(
                "vault transit decrypt status {}",
                resp.status()
            )));
        }
        let body: WrappedResp = resp
            .json()
            .await
            .map_err(|e| KmsError::Crypto(format!("decrypt body: {e}")))?;
        let plaintext_b64 = body
            .data
            .plaintext
            .ok_or_else(|| KmsError::Crypto("vault decrypt: no plaintext".into()))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(plaintext_b64)
            .map_err(|e| KmsError::Crypto(format!("decrypt b64: {e}")))?;
        Ok(Zeroizing::new(bytes))
    }

    #[instrument(skip(self, plaintext), level = "debug")]
    async fn wrap_dek(&self, plaintext: &[u8]) -> Result<Vec<u8>, KmsError> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(plaintext);
        let resp = self
            .http
            .post(self.encrypt_url())
            .header("X-Vault-Token", &self.token)
            .json(&EncryptReq { plaintext: b64 })
            .send()
            .await
            .map_err(|e| KmsError::Transport(format!("encrypt: {e}")))?;
        if !resp.status().is_success() {
            return Err(KmsError::Crypto(format!(
                "vault transit encrypt status {}",
                resp.status()
            )));
        }
        let body: WrappedResp = resp
            .json()
            .await
            .map_err(|e| KmsError::Crypto(format!("encrypt body: {e}")))?;
        let ciphertext = body
            .data
            .ciphertext
            .ok_or_else(|| KmsError::Crypto("vault encrypt: no ciphertext".into()))?;
        Ok(ciphertext.into_bytes())
    }

    fn id(&self) -> &'static str {
        "vault_transit"
    }
}
