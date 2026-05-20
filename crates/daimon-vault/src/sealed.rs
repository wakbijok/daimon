use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, OsRng, rand_core::RngCore},
};
use std::fmt;
use thiserror::Error;
use zeroize::Zeroizing;

/// Length of the XChaCha20-Poly1305 key (32 bytes).
pub const SEAL_KEY_LEN: usize = 32;
/// Length of the XChaCha20-Poly1305 nonce (24 bytes — extended nonce).
pub const SEAL_NONCE_LEN: usize = 24;

/// Symmetric sealing of secret blobs (D4 — Vaultwarden session token at rest).
///
/// The key comes from the `DAIMON_VAULT_SEAL_KEY` environment variable
/// (32 bytes, hex-encoded → 64 hex chars). The on-disk payload is
/// `nonce (24) || ciphertext` so a single file holds everything needed to
/// recover the plaintext given the key.
///
/// Disk-only theft yields nothing — the key is not on disk. Env + disk yields
/// the sealed payload, not the original Vaultwarden master password.
#[derive(Clone)]
pub struct SealedSession {
    cipher: XChaCha20Poly1305,
}

impl fmt::Debug for SealedSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Don't leak cipher state in Debug output.
        f.debug_struct("SealedSession").finish_non_exhaustive()
    }
}

impl SealedSession {
    /// Build from a 32-byte key.
    pub fn from_key(key: &[u8; SEAL_KEY_LEN]) -> Self {
        Self {
            cipher: XChaCha20Poly1305::new(key.into()),
        }
    }

    /// Read the key from the `DAIMON_VAULT_SEAL_KEY` env var (hex-encoded).
    pub fn from_env() -> Result<Self, SealError> {
        let hex_key = std::env::var("DAIMON_VAULT_SEAL_KEY")
            .map_err(|_| SealError::EnvKeyMissing)?;
        Self::from_hex(&hex_key)
    }

    /// Parse a hex-encoded 32-byte key.
    pub fn from_hex(hex_key: &str) -> Result<Self, SealError> {
        let bytes = hex::decode(hex_key.trim()).map_err(|_| SealError::InvalidHex)?;
        if bytes.len() != SEAL_KEY_LEN {
            return Err(SealError::InvalidKeyLength(bytes.len()));
        }
        let mut key = [0u8; SEAL_KEY_LEN];
        key.copy_from_slice(&bytes);
        let cipher = XChaCha20Poly1305::new((&key).into());
        // zero the temporary array
        Zeroizing::new(key);
        Ok(Self { cipher })
    }

    /// Seal a plaintext into `nonce (24) || ciphertext`.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>, SealError> {
        let mut nonce_bytes = [0u8; SEAL_NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| SealError::Encrypt(e.to_string()))?;
        let mut out = Vec::with_capacity(SEAL_NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Unseal a sealed blob (nonce-prefixed).
    pub fn unseal(&self, sealed: &[u8]) -> Result<Vec<u8>, SealError> {
        if sealed.len() < SEAL_NONCE_LEN {
            return Err(SealError::TruncatedPayload);
        }
        let (nonce_bytes, ciphertext) = sealed.split_at(SEAL_NONCE_LEN);
        let nonce = XNonce::from_slice(nonce_bytes);
        self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| SealError::Decrypt(e.to_string()))
    }

    /// Generate a fresh 32-byte key, hex-encoded — useful for bootstrap
    /// (operator runs `daimon vault gen-seal-key` once and pastes into the
    /// systemd Environment= line).
    pub fn generate_key_hex() -> String {
        let mut key = [0u8; SEAL_KEY_LEN];
        OsRng.fill_bytes(&mut key);
        hex::encode(key)
    }
}

#[derive(Debug, Error)]
pub enum SealError {
    #[error("DAIMON_VAULT_SEAL_KEY env var is not set")]
    EnvKeyMissing,
    #[error("seal key is not valid hex")]
    InvalidHex,
    #[error("seal key must be exactly 32 bytes (64 hex chars), got {0}")]
    InvalidKeyLength(usize),
    #[error("sealed payload truncated below nonce length")]
    TruncatedPayload,
    #[error("encryption failed: {0}")]
    Encrypt(String),
    #[error("decryption failed (wrong key or tampered payload): {0}")]
    Decrypt(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_recovers_plaintext() {
        let key = [42u8; SEAL_KEY_LEN];
        let session = SealedSession::from_key(&key);
        let plaintext = b"session token here";
        let sealed = session.seal(plaintext).unwrap();
        let recovered = session.unseal(&sealed).unwrap();
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn each_seal_uses_fresh_nonce() {
        let key = [42u8; SEAL_KEY_LEN];
        let session = SealedSession::from_key(&key);
        let plaintext = b"deterministic payload";
        let a = session.seal(plaintext).unwrap();
        let b = session.seal(plaintext).unwrap();
        // nonces (first 24 bytes) should differ -> ciphertext differs too
        assert_ne!(a[..SEAL_NONCE_LEN], b[..SEAL_NONCE_LEN]);
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let key_a = [42u8; SEAL_KEY_LEN];
        let key_b = [43u8; SEAL_KEY_LEN];
        let session_a = SealedSession::from_key(&key_a);
        let session_b = SealedSession::from_key(&key_b);
        let sealed = session_a.seal(b"secret").unwrap();
        let err = session_b.unseal(&sealed).unwrap_err();
        assert!(matches!(err, SealError::Decrypt(_)));
    }

    #[test]
    fn truncated_payload_returns_error() {
        let key = [42u8; SEAL_KEY_LEN];
        let session = SealedSession::from_key(&key);
        let err = session.unseal(&[0u8; 10]).unwrap_err();
        assert!(matches!(err, SealError::TruncatedPayload));
    }

    #[test]
    fn from_hex_parses_valid_key() {
        let hex_key = "0".repeat(64);
        SealedSession::from_hex(&hex_key).unwrap();
    }

    #[test]
    fn from_hex_rejects_short_key() {
        let err = SealedSession::from_hex(&"0".repeat(60)).unwrap_err();
        assert!(matches!(err, SealError::InvalidKeyLength(_)));
    }

    #[test]
    fn from_hex_rejects_invalid_chars() {
        let err = SealedSession::from_hex(&"z".repeat(64)).unwrap_err();
        assert!(matches!(err, SealError::InvalidHex));
    }

    #[test]
    fn generate_key_hex_yields_valid_key() {
        let hex_key = SealedSession::generate_key_hex();
        assert_eq!(hex_key.len(), 64);
        SealedSession::from_hex(&hex_key).unwrap();
    }
}
