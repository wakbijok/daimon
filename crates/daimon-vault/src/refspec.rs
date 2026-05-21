use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// A reference to a credential stored in the vault.
///
/// Three syntactic forms are accepted:
/// - `vault://<name>` — **named form (D22 default)**, looks up by `name` column
///   in the in-tree SQLite vault. This is the primary form for the in-tree vault.
/// - `vault://<organization>/<collection>/<item>` — path form, kept for
///   backward compatibility with the original Vaultwarden-shaped spec (D3,
///   superseded by D22). The in-tree vault resolves Path refs by treating
///   `item` as the credential name (the org/collection segments are ignored
///   but parsed successfully).
/// - `vault://<item-uuid>` — UUID form, parsed but not used by the in-tree
///   vault (which uses INTEGER ids). Kept for forward compatibility if an
///   external vault impl is ever bolted on.
///
/// The URL form is self-describing in logs and allows backends to be swapped
/// without changing the reference syntax in user code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum CredentialRef {
    /// `vault://<name>` — primary form for the in-tree vault (D22).
    Named(String),
    Path {
        organization: String,
        collection: String,
        item: String,
    },
    Uuid(String),
}

impl CredentialRef {
    pub fn parse(s: &str) -> Result<Self, RefParseError> {
        let rest = s
            .strip_prefix("vault://")
            .ok_or(RefParseError::MissingScheme)?;
        let parts: Vec<&str> = rest.split('/').collect();
        match parts.as_slice() {
            [single] if !single.is_empty() => {
                // UUID shape → Uuid variant; anything else → Named (D22).
                if looks_like_uuid(single) {
                    Ok(CredentialRef::Uuid((*single).to_owned()))
                } else {
                    Ok(CredentialRef::Named((*single).to_owned()))
                }
            }
            [org, coll, item] if !org.is_empty() && !coll.is_empty() && !item.is_empty() => {
                Ok(CredentialRef::Path {
                    organization: (*org).to_owned(),
                    collection: (*coll).to_owned(),
                    item: (*item).to_owned(),
                })
            }
            _ => Err(RefParseError::InvalidShape),
        }
    }

    /// The credential name this ref resolves to in the in-tree vault.
    /// Path refs resolve by the trailing `item` segment; UUID refs have no
    /// name (returns None).
    pub fn name(&self) -> Option<&str> {
        match self {
            CredentialRef::Named(n) => Some(n),
            CredentialRef::Path { item, .. } => Some(item),
            CredentialRef::Uuid(_) => None,
        }
    }
}

impl fmt::Display for CredentialRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CredentialRef::Named(name) => write!(f, "vault://{name}"),
            CredentialRef::Path {
                organization,
                collection,
                item,
            } => write!(f, "vault://{organization}/{collection}/{item}"),
            CredentialRef::Uuid(id) => write!(f, "vault://{id}"),
        }
    }
}

impl TryFrom<String> for CredentialRef {
    type Error = RefParseError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        CredentialRef::parse(&s)
    }
}

impl From<CredentialRef> for String {
    fn from(r: CredentialRef) -> Self {
        r.to_string()
    }
}

fn looks_like_uuid(s: &str) -> bool {
    // 8-4-4-4-12 hex. Not a strict parser; just a shape check.
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (i, b) in bytes.iter().enumerate() {
        if matches!(i, 8 | 13 | 18 | 23) {
            if *b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RefParseError {
    #[error("credential ref must begin with `vault://`")]
    MissingScheme,
    #[error("vault:// ref must be `vault://<name>`, `vault://<uuid>`, or `vault://<org>/<collection>/<item>`")]
    InvalidShape,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_path_form() {
        let r = CredentialRef::parse("vault://infra/network/mikrotik-edge").unwrap();
        assert_eq!(
            r,
            CredentialRef::Path {
                organization: "infra".into(),
                collection: "network".into(),
                item: "mikrotik-edge".into(),
            }
        );
    }

    #[test]
    fn parses_uuid_form() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let r = CredentialRef::parse(&format!("vault://{uuid}")).unwrap();
        assert_eq!(r, CredentialRef::Uuid(uuid.to_owned()));
    }

    #[test]
    fn rejects_missing_scheme() {
        let err = CredentialRef::parse("infra/network/mikrotik-edge").unwrap_err();
        assert_eq!(err, RefParseError::MissingScheme);
    }

    #[test]
    fn parses_named_form() {
        let r = CredentialRef::parse("vault://mikrotik-edge").unwrap();
        assert_eq!(r, CredentialRef::Named("mikrotik-edge".into()));
    }

    #[test]
    fn named_form_display_round_trip() {
        let r = CredentialRef::parse("vault://my-cred").unwrap();
        assert_eq!(r.to_string(), "vault://my-cred");
    }

    #[test]
    fn name_extracts_from_named() {
        let r = CredentialRef::parse("vault://my-cred").unwrap();
        assert_eq!(r.name(), Some("my-cred"));
    }

    #[test]
    fn name_extracts_trailing_item_from_path() {
        let r = CredentialRef::parse("vault://infra/network/mikrotik-edge").unwrap();
        assert_eq!(r.name(), Some("mikrotik-edge"));
    }

    #[test]
    fn name_returns_none_for_uuid() {
        let r = CredentialRef::parse("vault://12345678-1234-1234-1234-123456789abc").unwrap();
        assert_eq!(r.name(), None);
    }

    #[test]
    fn rejects_two_segments() {
        let err = CredentialRef::parse("vault://infra/network").unwrap_err();
        assert_eq!(err, RefParseError::InvalidShape);
    }

    #[test]
    fn rejects_four_segments() {
        let err = CredentialRef::parse("vault://infra/network/sub/item").unwrap_err();
        assert_eq!(err, RefParseError::InvalidShape);
    }

    #[test]
    fn display_round_trips_path() {
        let r = CredentialRef::parse("vault://infra/network/mikrotik-edge").unwrap();
        assert_eq!(r.to_string(), "vault://infra/network/mikrotik-edge");
    }

    #[test]
    fn display_round_trips_uuid() {
        let s = "vault://12345678-1234-1234-1234-123456789abc";
        let r = CredentialRef::parse(s).unwrap();
        assert_eq!(r.to_string(), s);
    }

    #[test]
    fn serde_round_trip() {
        let r = CredentialRef::parse("vault://infra/network/mikrotik-edge").unwrap();
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, "\"vault://infra/network/mikrotik-edge\"");
        let back: CredentialRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }
}
