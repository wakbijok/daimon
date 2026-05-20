use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// A reference to a credential stored in the vault.
///
/// Two syntactic forms are accepted:
/// - `vault://<organization>/<collection>/<item>` — path form, looks up by
///   organization name + collection name + item name within the vault hierarchy
/// - `vault://<item-uuid>` — direct form, references an item by its UUID
///
/// The URL form is self-describing in logs and allows non-Vaultwarden backends
/// (HashiCorp Vault, AWS Secrets Manager, etc.) to be plugged in later without
/// changing the reference syntax.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum CredentialRef {
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
                // Single segment must look like a UUID. Light validation — actual
                // UUID parsing is the vault client's job.
                if looks_like_uuid(single) {
                    Ok(CredentialRef::Uuid((*single).to_owned()))
                } else {
                    Err(RefParseError::SingleSegmentMustBeUuid)
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
}

impl fmt::Display for CredentialRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
    #[error("single-segment vault:// ref must be a UUID")]
    SingleSegmentMustBeUuid,
    #[error("vault:// ref must be either `vault://<uuid>` or `vault://<org>/<collection>/<item>`")]
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
    fn rejects_single_non_uuid_segment() {
        let err = CredentialRef::parse("vault://just-a-name").unwrap_err();
        assert_eq!(err, RefParseError::SingleSegmentMustBeUuid);
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
