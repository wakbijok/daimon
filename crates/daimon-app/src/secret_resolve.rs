//! P6 — resolve a `vault://` config reference to its plaintext THROUGH the
//! broker (D21: daimon-app never touches daimon-vault directly).
//!
//! Settings secrets are stored as `vault://<name>` refs (P6-3). A runtime that
//! needs the actual secret (an LLM API key, a gateway token) resolves the ref
//! here: `vault_list_metadata` → find by name → `vault_reveal` → the
//! `ApiToken` plaintext. Every reveal is audited by the broker.

#![cfg(feature = "ssr")]

use std::sync::Arc;

use daimon_broker::{Broker, Credential};

/// Resolve a vault credential by NAME to its `ApiToken` plaintext. Returns
/// `None` (with a log) if the credential is missing or is not an `ApiToken`.
pub async fn resolve_vault_api_token(
    broker: &Arc<Broker>,
    cred_name: &str,
    actor: &str,
) -> Option<String> {
    match broker.vault_list_metadata(actor).await {
        Ok(metas) => {
            if let Some(meta) = metas.into_iter().find(|m| m.name == cred_name) {
                match broker.vault_reveal(actor, meta.id).await {
                    // `Credential` is ZeroizeOnDrop — match by ref, clone the secret.
                    Ok(cred) => match &cred {
                        Credential::ApiToken { token } => return Some(token.clone()),
                        other => tracing::warn!(
                            cred = %cred_name,
                            kind = ?other.kind(),
                            "vault credential is not an ApiToken"
                        ),
                    },
                    Err(e) => tracing::warn!(cred = %cred_name, error = %e, "vault_reveal failed"),
                }
            } else {
                tracing::debug!(cred = %cred_name, "vault credential not found");
            }
        }
        Err(e) => tracing::warn!(error = %e, "vault_list_metadata failed"),
    }
    None
}

/// Resolve a config value that MAY be a `vault://<name>` ref to plaintext:
/// - `vault://<name>` → resolved through the broker (`None` if unresolvable);
/// - a non-empty plain value → returned as-is (dev plaintext, middle precedence);
/// - empty → `None`.
pub async fn resolve_maybe_ref(broker: &Arc<Broker>, value: &str, actor: &str) -> Option<String> {
    match value.strip_prefix("vault://") {
        Some(name) => resolve_vault_api_token(broker, name, actor).await,
        None if !value.is_empty() => Some(value.to_string()),
        None => None,
    }
}
