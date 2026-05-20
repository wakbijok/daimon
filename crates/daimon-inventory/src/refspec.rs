use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// Reference to a managed target — `target://<name>`.
///
/// Name is the registry key. Inventory resolves it to a `ManagedTarget`
/// record. Worker agents see only `TargetRef` and `TargetMetadata`; the full
/// record (including `credential_ref`) is broker-only.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TargetRef(String);

impl TargetRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn parse(s: &str) -> Result<Self, RefParseError> {
        let rest = s
            .strip_prefix("target://")
            .ok_or(RefParseError::MissingScheme)?;
        if rest.is_empty() {
            return Err(RefParseError::EmptyName);
        }
        if rest.contains('/') {
            return Err(RefParseError::InvalidShape);
        }
        Ok(Self(rest.to_owned()))
    }

    pub fn name(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TargetRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "target://{}", self.0)
    }
}

impl TryFrom<String> for TargetRef {
    type Error = RefParseError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        TargetRef::parse(&s)
    }
}

impl From<TargetRef> for String {
    fn from(r: TargetRef) -> Self {
        r.to_string()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RefParseError {
    #[error("target ref must begin with `target://`")]
    MissingScheme,
    #[error("target ref name is empty")]
    EmptyName,
    #[error("target ref must be a single name segment (no slashes)")]
    InvalidShape,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_name() {
        let r = TargetRef::parse("target://mikrotik-edge").unwrap();
        assert_eq!(r.name(), "mikrotik-edge");
        assert_eq!(r.to_string(), "target://mikrotik-edge");
    }

    #[test]
    fn rejects_missing_scheme() {
        assert_eq!(
            TargetRef::parse("mikrotik-edge").unwrap_err(),
            RefParseError::MissingScheme
        );
    }

    #[test]
    fn rejects_empty_name() {
        assert_eq!(
            TargetRef::parse("target://").unwrap_err(),
            RefParseError::EmptyName
        );
    }

    #[test]
    fn rejects_path_segments() {
        assert_eq!(
            TargetRef::parse("target://infra/mikrotik-edge").unwrap_err(),
            RefParseError::InvalidShape
        );
    }

    #[test]
    fn serde_round_trip() {
        let r = TargetRef::parse("target://nargothrond-pve").unwrap();
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, "\"target://nargothrond-pve\"");
        let back: TargetRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }
}
