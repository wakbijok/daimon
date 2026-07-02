//! Typed per-parameter character-class allowlist (FR-CON-12).
//!
//! This is the single injection chokepoint for every write daimon can emit.
//! `reject_shell_metachars` used to live locally in the RouterOS agent
//! (`crates/daimon-tool-network/src/lib.rs`, one ad-hoc allowlist
//! `[A-Za-z0-9._:/!@-]`); it is promoted here as a typed set of classes shared
//! by BOTH the code drivers and the generic `ConnectorDriver` renderer.
//!
//! Every capability's schema declares a `ParamClass` per parameter. A driver
//! MUST call [`validate`] on each supplied value BEFORE substituting it into
//! any `Op` template — the value is rejected on violation and no `Op` is built.
//! This is defence beyond the transport: `SshTransport` sends a command line as
//! one string with no shell in between on RouterOS, so client-side class
//! validation is the primary guard against a crafted param breaking the command
//! boundary.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The character class a parameter must satisfy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamClass {
    /// `[A-Za-z0-9 ._:/!@-]` — a strict superset of the RouterOS agent's
    /// historical `reject_shell_metachars` allowlist (`[A-Za-z0-9._:/!@-]`)
    /// plus a space, for free-text comments. Every shell metacharacter
    /// (`; | & $ ` < > ( ) { }`) is rejected.
    SafeText,
    /// An IP address or `ip/prefix` CIDR — digits, `.`, `:` (IPv6), and an
    /// optional `/nn` prefix suffix. Rejects anything else.
    Cidr,
    /// `[A-Za-z0-9_-]+` — interface names, node names, user names.
    Identifier,
    /// Parses as an `i64`.
    Int,
    /// Membership in a fixed set (e.g. `chain = forward | input | output`).
    Enum(Vec<String>),
}

/// Rejection reason from [`validate`]. Carries the offending char/value and the
/// class it violated so the caller can surface an actionable message.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParamError {
    #[error("empty parameter is not allowed")]
    Empty,
    #[error("value contains disallowed char `{ch}` for class {class} (value=`{value}`)")]
    DisallowedChar {
        ch: char,
        class: &'static str,
        value: String,
    },
    #[error("value `{value}` is not a valid {class}")]
    Malformed { value: String, class: &'static str },
    #[error("value `{value}` is not one of the allowed enum members {allowed:?}")]
    NotInEnum { value: String, allowed: Vec<String> },
}

/// Validate `value` against `class`. Returns `Ok(())` iff the value is safe to
/// substitute into an `Op` template. This is the injection chokepoint — call it
/// on every param BEFORE constructing the `Op`.
pub fn validate(value: &str, class: &ParamClass) -> Result<(), ParamError> {
    match class {
        ParamClass::SafeText => {
            if value.is_empty() {
                return Err(ParamError::Empty);
            }
            for c in value.chars() {
                // Superset of tool-network's `[A-Za-z0-9._:/!@-]` + space.
                let ok = c.is_ascii_alphanumeric()
                    || matches!(c, ' ' | '.' | '/' | '-' | '_' | ':' | '!' | '@');
                if !ok {
                    return Err(ParamError::DisallowedChar {
                        ch: c,
                        class: "SafeText",
                        value: value.to_owned(),
                    });
                }
            }
            Ok(())
        }
        ParamClass::Cidr => {
            if value.is_empty() {
                return Err(ParamError::Empty);
            }
            // Char-class gate: hex digits (IPv6 uses a-f), dots, colons
            // (IPv6), and a single `/` for the prefix suffix. This rejects
            // every shell metachar outright before we even look at the
            // structure.
            for c in value.chars() {
                let ok = c.is_ascii_hexdigit() || matches!(c, '.' | ':' | '/');
                if !ok {
                    return Err(ParamError::DisallowedChar {
                        ch: c,
                        class: "Cidr",
                        value: value.to_owned(),
                    });
                }
            }
            // Structural check: at most one `/`, and if present the suffix must
            // be a small non-negative integer (a plausible prefix length).
            let mut parts = value.splitn(2, '/');
            let addr = parts.next().unwrap_or("");
            if addr.is_empty() {
                return Err(ParamError::Malformed {
                    value: value.to_owned(),
                    class: "Cidr",
                });
            }
            if let Some(prefix) = parts.next() {
                match prefix.parse::<u8>() {
                    Ok(p) if p <= 128 => {}
                    _ => {
                        return Err(ParamError::Malformed {
                            value: value.to_owned(),
                            class: "Cidr",
                        });
                    }
                }
            }
            Ok(())
        }
        ParamClass::Identifier => {
            if value.is_empty() {
                return Err(ParamError::Empty);
            }
            for c in value.chars() {
                let ok = c.is_ascii_alphanumeric() || matches!(c, '_' | '-');
                if !ok {
                    return Err(ParamError::DisallowedChar {
                        ch: c,
                        class: "Identifier",
                        value: value.to_owned(),
                    });
                }
            }
            Ok(())
        }
        ParamClass::Int => value.parse::<i64>().map(|_| ()).map_err(|_| {
            ParamError::Malformed {
                value: value.to_owned(),
                class: "Int",
            }
        }),
        ParamClass::Enum(allowed) => {
            if allowed.iter().any(|a| a == value) {
                Ok(())
            } else {
                Err(ParamError::NotInEnum {
                    value: value.to_owned(),
                    allowed: allowed.clone(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shell metacharacter must be rejected by SafeText — this is the
    /// injection-chokepoint contract. Matches the historical
    /// `reject_shell_metachars` guarantee.
    #[test]
    fn safetext_rejects_every_shell_metachar() {
        for meta in [';', '|', '&', '$', '`', '<', '>', '(', ')', '{', '}'] {
            let value = format!("ok{meta}bad");
            let err = validate(&value, &ParamClass::SafeText).unwrap_err();
            match err {
                ParamError::DisallowedChar { ch, .. } => assert_eq!(ch, meta),
                other => panic!("expected DisallowedChar for `{meta}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn safetext_accepts_the_allowlist_superset() {
        // Every char in [A-Za-z0-9 ._:/!@-] plus mixed case + digits.
        validate("Az09 ._:/!@-", &ParamClass::SafeText).expect("allowlist superset must pass");
        validate("blocked by daimon", &ParamClass::SafeText).expect("comment with spaces");
    }

    #[test]
    fn safetext_rejects_empty() {
        assert_eq!(
            validate("", &ParamClass::SafeText).unwrap_err(),
            ParamError::Empty
        );
    }

    #[test]
    fn cidr_accepts_valid_and_rejects_invalid() {
        validate("192.168.1.0/24", &ParamClass::Cidr).expect("ipv4 cidr");
        validate("10.0.0.1", &ParamClass::Cidr).expect("bare ipv4");
        validate("2001:db8::1/64", &ParamClass::Cidr).expect("ipv6 cidr");
        // Injection attempt / hostnames / out-of-range prefix are rejected.
        // (Note: Cidr is a char-class + shape gate, not a full IP parser — it
        // bounds the prefix at 128 but does not distinguish v4/v6 ranges.)
        assert!(validate("10.0.0.1; rm -rf /", &ParamClass::Cidr).is_err());
        assert!(validate("10.0.0.0/999", &ParamClass::Cidr).is_err());
        assert!(validate("hostname", &ParamClass::Cidr).is_err());
    }

    #[test]
    fn identifier_accepts_valid_and_rejects_invalid() {
        validate("ether1-wan_0", &ParamClass::Identifier).expect("iface name");
        assert!(validate("ether1 wan", &ParamClass::Identifier).is_err()); // space
        assert!(validate("iface;drop", &ParamClass::Identifier).is_err()); // metachar
    }

    #[test]
    fn int_accepts_valid_and_rejects_invalid() {
        validate("-42", &ParamClass::Int).expect("negative int");
        validate("0", &ParamClass::Int).expect("zero");
        assert!(validate("12x", &ParamClass::Int).is_err());
        assert!(validate("", &ParamClass::Int).is_err());
    }

    #[test]
    fn enum_membership() {
        let chain = ParamClass::Enum(vec!["forward".into(), "input".into(), "output".into()]);
        validate("forward", &chain).expect("member");
        assert!(validate("prerouting", &chain).is_err());
    }
}
