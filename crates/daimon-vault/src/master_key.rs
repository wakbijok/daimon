//! Master key bootstrap for the in-tree credential vault (D22).
//!
//! The master key is a 32-byte secret used to wrap each row in the SQLite
//! vault via XChaCha20-Poly1305. It is bootstrapped from systemd's
//! `LoadCredentialEncrypted=` mechanism in production: systemd decrypts the
//! credstore file at service start and exposes the plaintext at
//! `$CREDENTIALS_DIRECTORY/vault-master`. daimon reads it once at startup,
//! holds it in a `MasterKey` (zeroize-on-drop), and never persists it
//! anywhere else.
//!
//! Operator install procedure (one-time):
//! ```text
//! head -c 32 /dev/urandom > /tmp/master.bin
//! systemd-creds encrypt --name=vault-master /tmp/master.bin \
//!     /etc/credstore.encrypted/daimon-vault-master
//! shred /tmp/master.bin
//! ```
//!
//! Then in `daimon.service`:
//! ```text
//! [Service]
//! LoadCredentialEncrypted=vault-master:/etc/credstore.encrypted/daimon-vault-master
//! ```
//!
//! Rotation: re-run the encrypt step with new key bytes, then re-encrypt
//! every row in the vault via `SqliteVaultClient::rotate_master_key`
//! (Phase 2c follow-up; not required for Phase 2b shipping).

use std::path::{Path, PathBuf};

use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::sealed::SEAL_KEY_LEN;

/// A 32-byte master key for the in-tree vault. Zeroized on drop.
#[derive(Zeroize, ZeroizeOnDrop, Clone)]
pub struct MasterKey {
    bytes: [u8; SEAL_KEY_LEN],
}

impl MasterKey {
    /// Construct from raw bytes. Used internally + by tests.
    pub fn from_bytes(bytes: [u8; SEAL_KEY_LEN]) -> Self {
        Self { bytes }
    }

    /// Borrow the key bytes — for handing to `SealedSession::from_key`.
    /// Callers must not store the slice past the lifetime of `&self`.
    pub fn as_bytes(&self) -> &[u8; SEAL_KEY_LEN] {
        &self.bytes
    }

    /// Read from systemd's credentials directory.
    ///
    /// Looks for `$CREDENTIALS_DIRECTORY/vault-master` (set by systemd when
    /// `LoadCredentialEncrypted=` is configured). Falls back to
    /// `/run/credentials/daimon.service/vault-master` if `$CREDENTIALS_DIRECTORY`
    /// is unset (e.g. during development).
    pub fn from_systemd_credential() -> Result<Self, MasterKeyError> {
        let path = match std::env::var("CREDENTIALS_DIRECTORY") {
            Ok(dir) => PathBuf::from(dir).join("vault-master"),
            Err(_) => PathBuf::from("/run/credentials/daimon.service/vault-master"),
        };
        Self::from_path(&path)
    }

    /// Read a 32-byte master key from a file. Errors if the file is missing,
    /// unreadable, or not exactly 32 bytes.
    pub fn from_path(path: &Path) -> Result<Self, MasterKeyError> {
        let bytes = std::fs::read(path).map_err(|e| MasterKeyError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        if bytes.len() != SEAL_KEY_LEN {
            return Err(MasterKeyError::InvalidLength {
                expected: SEAL_KEY_LEN,
                actual: bytes.len(),
            });
        }
        let mut key = [0u8; SEAL_KEY_LEN];
        key.copy_from_slice(&bytes);
        // Zero the temporary Vec — the bytes::Bytes will drop normally but
        // we want to be explicit about clearing any stray copies.
        let mut bytes = bytes;
        bytes.zeroize();
        Ok(Self { bytes: key })
    }
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MasterKey").finish_non_exhaustive()
    }
}

#[derive(Debug, Error)]
pub enum MasterKeyError {
    #[error("failed to read master key from `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("master key file has wrong length: expected {expected} bytes, got {actual}")]
    InvalidLength { expected: usize, actual: usize },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn from_bytes_stores_and_returns_bytes() {
        let raw = [7u8; SEAL_KEY_LEN];
        let key = MasterKey::from_bytes(raw);
        assert_eq!(key.as_bytes(), &raw);
    }

    #[test]
    fn from_path_reads_32_bytes() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        let bytes = [9u8; SEAL_KEY_LEN];
        tmp.write_all(&bytes).unwrap();
        tmp.flush().unwrap();
        let key = MasterKey::from_path(tmp.path()).unwrap();
        assert_eq!(key.as_bytes(), &bytes);
    }

    #[test]
    fn from_path_rejects_short_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&[0u8; 16]).unwrap();
        tmp.flush().unwrap();
        let err = MasterKey::from_path(tmp.path()).unwrap_err();
        assert!(matches!(
            err,
            MasterKeyError::InvalidLength { expected: 32, actual: 16 }
        ));
    }

    #[test]
    fn from_path_rejects_long_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&[0u8; 64]).unwrap();
        tmp.flush().unwrap();
        let err = MasterKey::from_path(tmp.path()).unwrap_err();
        assert!(matches!(
            err,
            MasterKeyError::InvalidLength { expected: 32, actual: 64 }
        ));
    }

    #[test]
    fn from_path_reports_missing_file() {
        let err = MasterKey::from_path(Path::new("/nonexistent/vault-master")).unwrap_err();
        assert!(matches!(err, MasterKeyError::Io { .. }));
    }

    #[test]
    fn debug_does_not_leak_key_bytes() {
        let key = MasterKey::from_bytes([0xAB; SEAL_KEY_LEN]);
        let dbg = format!("{key:?}");
        assert!(!dbg.contains("AB"));
        assert!(!dbg.contains("171")); // decimal 0xAB
        assert!(dbg.contains("MasterKey"));
    }
}
