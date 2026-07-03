//! Inbound authenticity verification (SDS §9.4.1 — FR-GW-07).
//!
//! Every inbound gateway request is verified BEFORE anything reaches the
//! Harness, the LLM, or the broker — the direct answer to the unauthenticated-
//! `/ws` lesson (C4). This module is the shared crypto toolkit each adapter uses:
//!
//! - **Telegram** — a shared secret token echoed in the
//!   `X-Telegram-Bot-Api-Secret-Token` header (registered at `setWebhook`);
//!   compared constant-time ([`verify_secret_token`]).
//! - **Slack / WhatsApp / generic webhook** (framework-ready) — HMAC-SHA256 over
//!   the raw body (+ timestamp for Slack) keyed by the signing secret
//!   ([`verify_hmac_sha256`]).
//!
//! All comparisons are **constant-time** ([`constant_time_eq`]) so a timing
//! side-channel cannot leak the expected secret byte-by-byte.

use subtle::ConstantTimeEq;

/// Constant-time byte-slice equality. Lengths are compared first (a message's
/// length is not secret); the content comparison is constant-time.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

/// Verify a shared secret token (Telegram's `X-Telegram-Bot-Api-Secret-Token`).
/// `provided` is the header value from the inbound request (absent → reject).
pub fn verify_secret_token(expected: &str, provided: Option<&str>) -> bool {
    match provided {
        Some(p) => constant_time_eq(expected.as_bytes(), p.as_bytes()),
        None => false,
    }
}

/// HMAC-SHA256 hex digest of `msg` keyed by `secret`. Lowercase hex, matching
/// the wire format Slack (`v0=<hex>`) and WhatsApp (`sha256=<hex>`) use.
pub fn hmac_sha256_hex(secret: &[u8], msg: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(msg);
    hex::encode(mac.finalize().into_bytes())
}

/// Verify an HMAC-SHA256 hex signature over `msg`, constant-time. `provided_hex`
/// is the bare hex digest (strip any `v0=` / `sha256=` prefix before calling).
pub fn verify_hmac_sha256(secret: &[u8], msg: &[u8], provided_hex: &str) -> bool {
    let expected = hmac_sha256_hex(secret, msg);
    constant_time_eq(expected.as_bytes(), provided_hex.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_and_rejects() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreu"));
        assert!(!constant_time_eq(b"secret", b"secret-longer"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn secret_token_accept_reject() {
        assert!(verify_secret_token("tok-abc123", Some("tok-abc123")));
        assert!(!verify_secret_token("tok-abc123", Some("tok-wrong")));
        assert!(!verify_secret_token("tok-abc123", None));
    }

    #[test]
    fn hmac_sha256_known_vector() {
        // Widely-published HMAC-SHA256 test vector: key="key",
        // msg="The quick brown fox jumps over the lazy dog".
        let digest = hmac_sha256_hex(
            b"key",
            b"The quick brown fox jumps over the lazy dog",
        );
        assert_eq!(
            digest,
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
    }

    #[test]
    fn hmac_verify_accept_reject() {
        let secret = b"signing-secret";
        let body = br#"{"event":"ping"}"#;
        let good = hmac_sha256_hex(secret, body);
        assert!(verify_hmac_sha256(secret, body, &good));
        assert!(!verify_hmac_sha256(secret, body, "deadbeef"));
        assert!(!verify_hmac_sha256(b"wrong-secret", body, &good));
    }
}
