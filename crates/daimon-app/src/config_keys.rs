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
    (
        "identity.org_name",
        "Organisation name shown in the console header",
    ),
    // LLM (P6-4)
    (
        "llm.provider",
        "Active LLM provider: anthropic | openai | chatgpt | local",
    ),
    ("llm.default_model.chat", "Model for the chat/worker role"),
    (
        "llm.available_models",
        "Comma-separated models an operator may pick in chat (unset = default only)",
    ),
    (
        "llm.anthropic_key",
        "Anthropic API key (secret → vault ref)",
    ),
    ("llm.openai_key", "OpenAI API key (secret → vault ref)"),
    ("llm.ollama_url", "Base URL for the local/Ollama provider"),
    // Guard (P6-5)
    (
        "guard.approval_timeout_secs",
        "Seconds to await an approval before denying",
    ),
    (
        "guard.blast_radius_depth",
        "Graph traversal depth for the approval blast radius",
    ),
    // Observer (P6-5)
    (
        "observer.prom_poll_interval_secs",
        "Prometheus poll interval (seconds)",
    ),
    // Channels (P4 + P6-10)
    ("channels.telegram.enabled", "Enable the Telegram gateway"),
    (
        "channels.telegram.mode",
        "Telegram ingress mode: poll | webhook",
    ),
    (
        "channels.telegram.bot_token_cred",
        "Telegram bot token (secret → vault ref)",
    ),
    (
        "channels.telegram.webhook_secret_cred",
        "Telegram webhook signing secret credential (webhook mode only)",
    ),
    (
        "channels.telegram.offset",
        "Telegram getUpdates offset (runtime cursor)",
    ),
    (
        "channels.matrix.",
        "Matrix gateway configuration (enabled, homeserver, token cred, …)",
    ),
    (
        "channels.alerts.",
        "Outbound alert routing rules (by class + severity → recipient)",
    ),
    // Targets/Connectors (P6-7)
    (
        "targets.",
        "Registered managed targets (target://<name>) + driver/connector binding",
    ),
    // Chat history retention (P7-6)
    (
        "chat.history_retention_days",
        "Days to retain chat transcripts (0 = forever); independent of the auth-session TTL",
    ),
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

/// The settings domains the boot config-coherence lint (FR-CFG-01) enforces:
/// every `app_config` key under one of these prefixes MUST be either consumed by
/// the runtime or read-only, else it is an orphan editable key (write-only
/// theatre). Prefixes NOT listed here (`jwt_secret`, `update.*`, `rag.*` — the
/// latter only partially wired) are exempt so the lint asserts only what we
/// claim is wired.
pub const MANAGED_LINT_PREFIXES: &[&str] = &[
    "identity.",
    "llm.",
    "guard.",
    "observer.",
    "channels.",
    "targets.",
];

/// Orphan editable keys: keys under a managed prefix that the runtime neither
/// consumes nor renders read-only (FR-CFG-01). An empty result = coherent.
pub fn orphan_keys<'a>(all_keys: impl Iterator<Item = &'a str>) -> Vec<String> {
    all_keys
        .filter(|k| {
            MANAGED_LINT_PREFIXES.iter().any(|p| k.starts_with(p))
                && !is_consumed(k)
                && !is_read_only_key(k)
        })
        .map(str::to_string)
        .collect()
}

/// Render the code-derived configuration reference (FR-CFG-04): every consumed
/// key + descriptor and the read-only domains, as Markdown. Derived from the
/// registry, so it cannot drift from what the runtime actually reads.
pub fn render_reference() -> String {
    let mut out = String::from("# daimon configuration reference\n\n");
    out.push_str(
        "Resolution precedence for every key: **DB `app_config` (operator edit) → \
         environment variable → compiled default** (FR-CFG-02). Bootstrap secrets \
         (`DAIMON_PG_URL`, master key, `DAIMON_DATA_DIR`) are env/credential-sourced \
         and never in `app_config` (FR-CFG-03).\n\n",
    );
    out.push_str("## Consumed keys (read by the runtime)\n\n");
    out.push_str("| key | description |\n|-----|-------------|\n");
    for (k, desc) in CONSUMED_KEYS {
        out.push_str(&format!("| `{k}` | {desc} |\n"));
    }
    out.push_str("\n## Read-only domains (not runtime-consumed in this build)\n\n");
    for p in READ_ONLY_PREFIXES {
        out.push_str(&format!("- `{p}*`\n"));
    }
    out
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

    /// P7-10 (FR-CFG-04): the committed config reference must match the
    /// code-derived render — CI drift gate. Regenerate with:
    ///   DAIMON_UPDATE_DOCS=1 cargo test -p daimon-app config_reference_doc_is_current
    #[test]
    fn config_reference_doc_is_current() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/config-reference.md"
        );
        let expected = super::render_reference();
        if std::env::var("DAIMON_UPDATE_DOCS").is_ok() {
            std::fs::write(path, &expected).expect("write config-reference.md");
            return;
        }
        let actual = std::fs::read_to_string(path).unwrap_or_default();
        assert_eq!(
            actual, expected,
            "docs/config-reference.md is stale — regenerate: DAIMON_UPDATE_DOCS=1 cargo test -p daimon-app config_reference_doc_is_current"
        );
    }
}
