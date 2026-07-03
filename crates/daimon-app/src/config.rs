//! P6 — the single configuration source-of-truth resolver (FR-CFG-02/03/14/15).
//!
//! Before P6 the runtime read config incoherently: a raw `std::env::var` in one
//! place, an `app_config` row in another, and several `/settings` fields that
//! nothing read at all (the "write-only theatre" defect, H13). [`ConfigResolver`]
//! fixes that by resolving EVERY runtime config read through one deterministic
//! precedence:
//!
//! > **DB `app_config` (operator edit) → environment variable → compiled default**
//!
//! so an operator edit in the console wins over a stale unit-file env, which
//! wins over the built-in default. The snapshot is held behind an [`ArcSwap`],
//! so a settings write hot-swaps a fresh snapshot in (FR-CFG-14) and concurrent
//! readers never block and never see a torn value.
//!
//! Two hard rules:
//! - **Bootstrap secrets are NOT resolver-backed** (FR-CFG-03): `DAIMON_PG_URL`,
//!   the master key, and `DAIMON_DATA_DIR` must be reachable *before* Postgres is
//!   open, so they stay env/credential-sourced and never route through here.
//! - **A malformed DB value never panics** (FR-CFG-15): if an `app_config` row is
//!   present but not the shape the accessor wants, the resolver logs a warning
//!   and falls through to env/default, so one bad JSONB row can't take the
//!   runtime down.
//!
//! The resolver holds NO vault/inventory/transport handle (D21): a `is_secret`
//! row surfaces its `vault://` ref string verbatim; whoever needs the plaintext
//! asks the broker to resolve the ref.

#![cfg(feature = "ssr")]

use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use serde_json::Value as Json;

/// An immutable snapshot of `public.app_config`, loaded at boot and on every
/// settings write. Accessors apply the DB → env → default precedence.
#[derive(Debug, Default)]
pub struct ConfigSnapshot {
    map: HashMap<String, Json>,
}

impl ConfigSnapshot {
    /// Build a snapshot from raw `(key, value)` pairs. Used by the loader and by
    /// tests; production builds it from an `app_config` query.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, Json)>) -> Self {
        Self {
            map: pairs.into_iter().collect(),
        }
    }

    /// A `String` with DB → env → default precedence. A DB row that is present
    /// but not a JSON string is MALFORMED: warn + fall through (FR-CFG-15).
    pub fn string(&self, key: &str, env: Option<&str>, default: &str) -> String {
        self.opt_string(key, env)
            .unwrap_or_else(|| default.to_string())
    }

    /// Like [`string`](Self::string) but with no compiled default — `None` when
    /// neither the DB nor the env supplies a value (e.g. an optional API key).
    pub fn opt_string(&self, key: &str, env: Option<&str>) -> Option<String> {
        if let Some(v) = self.map.get(key) {
            match v.as_str() {
                Some(s) if !s.is_empty() => return Some(s.to_string()),
                Some(_) => {} // empty string in DB → treat as unset, fall through
                None => tracing::warn!(
                    key,
                    "app_config value is not a JSON string; falling back to env/default"
                ),
            }
        }
        env.and_then(|e| std::env::var(e).ok())
            .filter(|s| !s.is_empty())
    }

    /// A `u64` with DB → env → default precedence. The DB value may be a JSON
    /// number OR a JSON string; a value that parses as neither is malformed and
    /// falls through (FR-CFG-15).
    pub fn u64(&self, key: &str, env: Option<&str>, default: u64) -> u64 {
        if let Some(v) = self.map.get(key) {
            if let Some(n) = v.as_u64() {
                return n;
            }
            if let Some(n) = v.as_str().and_then(|s| s.parse::<u64>().ok()) {
                return n;
            }
            tracing::warn!(key, "app_config value is not a u64; falling back to env/default");
        }
        if let Some(n) = env
            .and_then(|e| std::env::var(e).ok())
            .and_then(|s| s.parse::<u64>().ok())
        {
            return n;
        }
        default
    }

    /// A `bool` with DB → env → default precedence. Accepts a JSON bool or the
    /// strings `"true"`/`"false"` (case-insensitive).
    pub fn bool(&self, key: &str, env: Option<&str>, default: bool) -> bool {
        if let Some(v) = self.map.get(key) {
            if let Some(b) = v.as_bool() {
                return b;
            }
            if let Some(b) = v.as_str().and_then(parse_bool) {
                return b;
            }
            tracing::warn!(key, "app_config value is not a bool; falling back to env/default");
        }
        if let Some(b) = env
            .and_then(|e| std::env::var(e).ok())
            .and_then(|s| parse_bool(&s))
        {
            return b;
        }
        default
    }

    /// The raw JSON value for a key, if present (no env/default fallback). Used
    /// by consumers that need a structured value, e.g. the alert routing table.
    pub fn raw(&self, key: &str) -> Option<&Json> {
        self.map.get(key)
    }

    /// Every `(key, value)` whose key starts with `prefix`. Used to read a whole
    /// config domain at once (e.g. `channels.alerts.` routing rules).
    pub fn under_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = (&'a String, &'a Json)> {
        self.map.iter().filter(move |(k, _)| k.starts_with(prefix))
    }

    /// Every loaded key (for the boot config-coherence lint, P6-13).
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(String::as_str)
    }

    /// Number of loaded keys (diagnostics / boot log).
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// The live config handle held in `AppState`. Wraps an [`ArcSwap`] so a reader
/// takes a consistent snapshot with `current()` and a writer swaps a fresh one
/// with `reload()` — lock-free on the read path.
pub struct ConfigResolver {
    inner: ArcSwap<ConfigSnapshot>,
}

impl ConfigResolver {
    /// Load the initial snapshot from `app_config`.
    pub async fn load(db: &daimon_db::Pool) -> anyhow::Result<Self> {
        let snap = load_snapshot(db).await?;
        Ok(Self {
            inner: ArcSwap::from_pointee(snap),
        })
    }

    /// Build a resolver directly from a snapshot (tests / degraded boot).
    pub fn from_snapshot(snap: ConfigSnapshot) -> Self {
        Self {
            inner: ArcSwap::from_pointee(snap),
        }
    }

    /// Re-read `app_config` and atomically swap the snapshot in (FR-CFG-14).
    /// Called at the tail of a settings write; a reload error is surfaced to the
    /// caller so the write path can log it WITHOUT failing the save.
    pub async fn reload(&self, db: &daimon_db::Pool) -> anyhow::Result<()> {
        let snap = load_snapshot(db).await?;
        self.inner.store(Arc::new(snap));
        Ok(())
    }

    /// Take a consistent snapshot for a burst of reads.
    pub fn current(&self) -> Arc<ConfigSnapshot> {
        self.inner.load_full()
    }
}

async fn load_snapshot(db: &daimon_db::Pool) -> anyhow::Result<ConfigSnapshot> {
    let client = db.get().await?;
    let rows = client
        .query("SELECT key, value FROM public.app_config", &[])
        .await?;
    let pairs = rows
        .into_iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, Json>(1)));
    Ok(ConfigSnapshot::from_pairs(pairs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snap(pairs: &[(&str, Json)]) -> ConfigSnapshot {
        ConfigSnapshot::from_pairs(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())))
    }

    #[test]
    fn string_db_wins_over_env_and_default() {
        // SAFETY: single-threaded test; set a unique env var.
        unsafe { std::env::set_var("DAIMON_TEST_CFG_STR", "from-env") };
        let s = snap(&[("llm.default_model.chat", json!("from-db"))]);
        assert_eq!(
            s.string("llm.default_model.chat", Some("DAIMON_TEST_CFG_STR"), "from-default"),
            "from-db"
        );
        unsafe { std::env::remove_var("DAIMON_TEST_CFG_STR") };
    }

    #[test]
    fn string_env_wins_when_db_absent() {
        unsafe { std::env::set_var("DAIMON_TEST_CFG_STR2", "from-env") };
        let s = snap(&[]);
        assert_eq!(
            s.string("missing.key", Some("DAIMON_TEST_CFG_STR2"), "from-default"),
            "from-env"
        );
        unsafe { std::env::remove_var("DAIMON_TEST_CFG_STR2") };
    }

    #[test]
    fn string_default_when_neither() {
        let s = snap(&[]);
        assert_eq!(s.string("missing.key", Some("DAIMON_TEST_UNSET_XYZ"), "d"), "d");
        assert_eq!(s.string("missing.key", None, "d"), "d");
    }

    #[test]
    fn malformed_db_value_falls_through_not_panics() {
        // FR-CFG-15: a number where a string is expected → fall through, no panic.
        let s = snap(&[("llm.model", json!(42))]);
        assert_eq!(s.string("llm.model", None, "safe-default"), "safe-default");
        // empty string in DB is treated as unset.
        let s2 = snap(&[("llm.model", json!(""))]);
        assert_eq!(s2.string("llm.model", None, "safe-default"), "safe-default");
    }

    #[test]
    fn u64_accepts_number_or_string_else_falls_through() {
        assert_eq!(snap(&[("guard.timeout", json!(30))]).u64("guard.timeout", None, 60), 30);
        assert_eq!(snap(&[("guard.timeout", json!("45"))]).u64("guard.timeout", None, 60), 45);
        // malformed → default
        assert_eq!(snap(&[("guard.timeout", json!("abc"))]).u64("guard.timeout", None, 60), 60);
        assert_eq!(snap(&[]).u64("guard.timeout", None, 60), 60);
    }

    #[test]
    fn bool_accepts_json_bool_or_string() {
        assert!(snap(&[("f.on", json!(true))]).bool("f.on", None, false));
        assert!(snap(&[("f.on", json!("yes"))]).bool("f.on", None, false));
        assert!(!snap(&[("f.on", json!("off"))]).bool("f.on", None, true));
        assert!(snap(&[]).bool("f.on", None, true));
    }

    #[test]
    fn under_prefix_collects_a_domain() {
        let s = snap(&[
            ("channels.alerts.anomaly.critical", json!("tg:66784431")),
            ("channels.alerts.approval.high", json!("tg:66784431")),
            ("llm.model", json!("x")),
        ]);
        let mut keys: Vec<_> = s
            .under_prefix("channels.alerts.")
            .map(|(k, _)| k.clone())
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "channels.alerts.anomaly.critical".to_string(),
                "channels.alerts.approval.high".to_string()
            ]
        );
    }
}
