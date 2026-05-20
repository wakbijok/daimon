use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A resolved credential. Variants cover the credential shapes daimon agents
/// need today; `Generic` carries arbitrary key/value fields for unusual cases.
///
/// `Debug` is implemented manually to redact secret material — never log the
/// raw `Credential`.
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Credential {
    /// An SSH private key (PEM-encoded). Optional passphrase.
    SshKey {
        username: String,
        private_key_pem: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        passphrase: Option<String>,
    },
    /// Password-based SSH (treated separately so worker agents can route via
    /// auth method without inspecting `Generic` blobs).
    SshPassword { username: String, password: String },
    /// API bearer / personal access token (REST APIs, vendor portals, etc.).
    ApiToken { token: String },
    /// Fallback bag of fields for credentials that don't fit the typed variants.
    /// BTreeMap doesn't impl Zeroize — fields are skipped from zero-on-drop.
    /// Use the typed variants (SshKey/SshPassword/ApiToken) where possible to
    /// get full memory wipe on drop.
    Generic {
        #[zeroize(skip)]
        fields: BTreeMap<String, String>,
    },
}

/// Discriminator-only enum, useful for logging / routing without exposing
/// secrets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    SshKey,
    SshPassword,
    ApiToken,
    Generic,
}

impl Credential {
    pub fn kind(&self) -> CredentialKind {
        match self {
            Credential::SshKey { .. } => CredentialKind::SshKey,
            Credential::SshPassword { .. } => CredentialKind::SshPassword,
            Credential::ApiToken { .. } => CredentialKind::ApiToken,
            Credential::Generic { .. } => CredentialKind::Generic,
        }
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print secret bytes — even if a tracing config accidentally
        // routes Credential into a log line, it shows only the kind.
        match self {
            Credential::SshKey { username, .. } => f
                .debug_struct("SshKey")
                .field("username", username)
                .field("private_key_pem", &"<redacted>")
                .field("passphrase", &"<redacted>")
                .finish(),
            Credential::SshPassword { username, .. } => f
                .debug_struct("SshPassword")
                .field("username", username)
                .field("password", &"<redacted>")
                .finish(),
            Credential::ApiToken { .. } => f
                .debug_struct("ApiToken")
                .field("token", &"<redacted>")
                .finish(),
            Credential::Generic { fields } => f
                .debug_struct("Generic")
                .field(
                    "fields",
                    &fields.keys().map(|k| (k.as_str(), "<redacted>")).collect::<BTreeMap<_, _>>(),
                )
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_key_serde_round_trip() {
        let c = Credential::SshKey {
            username: "arif".into(),
            private_key_pem: "-----BEGIN OPENSSH PRIVATE KEY-----\nfake\n-----END OPENSSH PRIVATE KEY-----".into(),
            passphrase: None,
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Credential = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind(), CredentialKind::SshKey);
    }

    #[test]
    fn debug_redacts_secret_bytes() {
        let c = Credential::SshPassword {
            username: "arif".into(),
            password: "DO_NOT_LEAK_THIS".into(),
        };
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("DO_NOT_LEAK_THIS"));
        assert!(dbg.contains("<redacted>"));
        assert!(dbg.contains("arif"));
    }

    #[test]
    fn debug_redacts_generic_field_values() {
        let mut fields = BTreeMap::new();
        fields.insert("api_secret".to_owned(), "SECRET_VALUE".to_owned());
        let c = Credential::Generic { fields };
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("SECRET_VALUE"));
        assert!(dbg.contains("api_secret"));
    }

    #[test]
    fn kind_distinguishes_variants() {
        let key_cred = Credential::SshKey {
            username: "u".into(),
            private_key_pem: "k".into(),
            passphrase: None,
        };
        let pw_cred = Credential::SshPassword {
            username: "u".into(),
            password: "p".into(),
        };
        assert_ne!(key_cred.kind(), pw_cred.kind());
    }
}
