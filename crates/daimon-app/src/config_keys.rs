//! P6 — the consumed-key registry (FR-CFG-01), the single source of truth for
//! which `app_config` keys the runtime actually READS.
//!
//! Pure data + helpers (no server deps) so BOTH the settings UI (wasm/hydrate)
//! and the server-side boot config-coherence lint (P6-13) reference the same
//! list. The UI shows these as the consumed keys per tab (so an operator knows
//! what is live vs decorative); the boot lint asserts every non-read-only
//! editable key is on this list — no field that silently does nothing.
//!
//! An entry ending in `.` matches by PREFIX (a whole sub-domain, e.g. every
//! `channels.alerts.<class>.<severity>` routing rule); otherwise it is an exact
//! key. Each carries a one-line descriptor for the UI + generated reference
//! (FR-CFG-04).

/// `(key_or_prefix, description)`. Prefix entries end in `.`.
pub const CONSUMED_KEYS: &[(&str, &str)] = &[
    // Identity
    ("identity.org_name", "Organisation name shown in the console header"),
    // LLM (P6-4)
    ("llm.provider", "Active LLM provider: anthropic | openai | chatgpt | local"),
    ("llm.default_model.chat", "Model for the chat/worker role"),
    ("llm.anthropic_key", "Anthropic API key (secret → vault ref)"),
    ("llm.openai_key", "OpenAI API key (secret → vault ref)"),
    ("llm.ollama_url", "Base URL for the local/Ollama provider"),
    // Guard (P6-5)
    ("guard.approval_timeout_secs", "Seconds to await an approval before denying"),
    ("guard.blast_radius_depth", "Graph traversal depth for the approval blast radius"),
    // Observer (P6-5)
    ("observer.prom_poll_interval_secs", "Prometheus poll interval (seconds)"),
    // Channels (P4 + P6-10)
    ("channels.telegram.enabled", "Enable the Telegram gateway"),
    ("channels.telegram.mode", "Telegram ingress mode: poll | webhook"),
    ("channels.telegram.bot_token_cred", "Telegram bot token (secret → vault ref)"),
    ("channels.telegram.offset", "Telegram getUpdates offset (runtime cursor)"),
    ("channels.matrix.", "Matrix gateway configuration (enabled, homeserver, token cred, …)"),
    ("channels.alerts.", "Outbound alert routing rules (by class + severity → recipient)"),
    // Targets/Connectors (P6-7)
    ("targets.", "Registered managed targets (target://<name>) + driver/connector binding"),
];

/// Config-tab prefixes whose keys are NOT consumed by the runtime in this build
/// and are therefore rendered READ-ONLY (FR-CFG-01/03/10): `connections.*` is
/// bootstrap/env-sourced (FR-CFG-03) and `vault.*` is KMS-dead in revival scope.
pub const READ_ONLY_PREFIXES: &[&str] = &["connections.", "vault."];

/// True if `key` is consumed by the runtime (exact or prefix match).
pub fn is_consumed(key: &str) -> bool {
    CONSUMED_KEYS.iter().any(|(k, _)| {
        if let Some(prefix) = k.strip_suffix('.') {
            key.starts_with(prefix) && key.len() > prefix.len()
        } else {
            key == *k
        }
    })
}

/// True if `key` falls under a read-only settings prefix.
pub fn is_read_only_key(key: &str) -> bool {
    READ_ONLY_PREFIXES.iter().any(|p| key.starts_with(p))
}

/// Consumed keys under a given tab prefix, for the settings UI hint.
pub fn consumed_keys_under(prefix: &str) -> Vec<(&'static str, &'static str)> {
    CONSUMED_KEYS
        .iter()
        .filter(|(k, _)| k.starts_with(prefix))
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_and_prefix_matching() {
        assert!(is_consumed("llm.provider"));
        assert!(is_consumed("channels.alerts.anomaly.critical")); // prefix
        assert!(is_consumed("channels.matrix.enabled")); // prefix
        assert!(!is_consumed("llm.")); // bare prefix is not itself a key
        assert!(!is_consumed("guard.nonsense")); // unknown → not consumed
    }

    #[test]
    fn read_only_prefixes_classify() {
        assert!(is_read_only_key("connections.pg_url"));
        assert!(is_read_only_key("vault.kms_path"));
        assert!(!is_read_only_key("llm.provider"));
    }
}
