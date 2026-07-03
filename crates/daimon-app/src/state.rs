#[cfg(feature = "ssr")]
use std::sync::Arc;

#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct AppState {
    /// Phase 2c D3b: Postgres pool. Replaces the prior
    /// `Arc<Mutex<rusqlite::Connection>>`.
    pub db: daimon_db::Pool,
    pub jwt_secret: String,
    pub ws_broadcast: tokio::sync::broadcast::Sender<String>,
    /// Phase 2b #19: action broker — the single integration point between
    /// daimon-app server-fns and the D19/D20/D22/D23/D24 stack
    /// (vault + inventory + transport + audit). Constructed once at boot
    /// in `main.rs::boot_broker()` and shared across all request handlers.
    pub broker: Arc<daimon_broker::Broker>,
    /// P2 — the multi-agent harness (bus + capability registry + supervisor).
    /// Chat + orchestrator dispatch capability calls over the bus through this,
    /// resolving the provider via the registry (versioned, fail-closed). The
    /// first production consumer of daimon-runtime. Replaced the pre-P2 direct
    /// `network_agent` path in P2 commit 5.
    pub harness: crate::harness::Harness,
    /// Phase 4 D4 — hot working memory tier (Redis in prod; in-proc fallback
    /// when Redis is unavailable). Used by the chat handler for session
    /// persistence + by Phase 5 for the kill-switch signal channel.
    pub working_memory: Arc<dyn daimon_redis::WorkingMemory>,
    /// Phase 6 D1 — orchestrator. Owns plan persistence + topological
    /// execution. Admin UI calls list_plans / create_plan / run_plan
    /// through this.
    pub orchestrator: Arc<daimon_orchestrator::OrchestratorService>,
    /// Phase 8 — graph tier (NornicDB). Optional: `None` when
    /// `DAIMON_GRAPH_URL` isn't set or the daemon was unreachable at boot.
    /// Used by /admin/approvals to render blast-radius summaries and by
    /// the orchestrator to mirror plan DAGs.
    pub graph: Option<Arc<dyn daimon_graph::GraphClient>>,
    /// P3 — long-term memory tier. A dmem SIDECAR (dm-lite `dmem serve`)
    /// reached over bearer-authenticated HTTP behind the `MemoryService`
    /// trait (LOCKED: musl-static cannot embed zvec's native lib). Chat
    /// pre-turn recall, chat memory-tool captures, and /admin/memory route
    /// through this. Falls back to `NullMemory` (degrades, never fails boot)
    /// when the sidecar is unconfigured or unreachable at boot.
    pub memory: Arc<dyn daimon_memory::MemoryService>,
    /// P3 commit 11 — daimon's OWN self-metrics (AC-P3-06). Hand-rolled
    /// `AtomicU64` counters (no `prometheus`/protobuf dep — musl-size), rendered
    /// as Prometheus text by `/metrics`. The three observer-owned counters are
    /// shared `Arc<AtomicU64>` handles the `ObserverIngest` increments directly,
    /// so `/metrics` is a single source of truth without an observer→app dep.
    pub self_metrics: Arc<crate::observability::SelfMetrics>,
    /// P4 — messaging gateways (SRS §4.8). The enabled webhook adapters keyed by
    /// channel id; the `POST /api/v1/gw/{channel}` route dispatches through this.
    /// Populated at boot from the Channels config (`build_registry`, P4-7); empty
    /// when no channel is enabled — the route then 404s and no adapter runs.
    pub gateways: Arc<crate::gw::GatewayRegistry>,
    /// P5-5 — skills (workflow-templates). Loaded from `deploy/skills/*.toml` at
    /// boot; running one instantiates a plan through the orchestrator.
    pub skills: Arc<crate::skills::SkillLibrary>,
    /// P6 — the single config source-of-truth resolver (FR-CFG-02). Every
    /// runtime config read resolves through this (DB `app_config` → env →
    /// default); a settings write calls `reload()` to hot-swap a fresh snapshot
    /// (FR-CFG-14). Held as `Arc` so background tasks (observer, router) share
    /// the same live handle.
    pub config: Arc<crate::config::ConfigResolver>,
    /// P6 (FR-CFG-10) — the observer poll interval (seconds), shared live with
    /// the observer ingest loop. `apply_runtime_tunables` writes it from config;
    /// the loop re-reads it each tick.
    pub observer_interval_secs: Arc<std::sync::atomic::AtomicU64>,
}

#[cfg(feature = "ssr")]
impl AppState {
    /// P6 (FR-CFG-06/10): push the live-tunable config values into the running
    /// subsystems — the guard's approval timeout (via the broker) and the
    /// observer poll interval (via the shared atomic). Called at boot and at the
    /// tail of every settings write, so an operator edit applies without a
    /// restart. Safety-critical guard flags (is_read_only / irreversible /
    /// compensating) are deliberately NOT here — they are server-derived, never
    /// config (FR-CFG-06).
    pub fn apply_runtime_tunables(&self, cfg: &crate::config::ConfigSnapshot) {
        use std::sync::atomic::Ordering;
        let timeout = cfg.u64(
            "guard.approval_timeout_secs",
            Some("DAIMON_APPROVAL_TIMEOUT_SECS"),
            daimon_guard::DEFAULT_APPROVAL_TIMEOUT_SECS,
        );
        if let Some(g) = self.broker.guard() {
            g.set_approval_timeout_secs(timeout);
        }
        let interval = cfg.u64(
            "observer.prom_poll_interval_secs",
            Some("DAIMON_PROM_POLL_INTERVAL_SECS"),
            30,
        );
        self.observer_interval_secs.store(interval, Ordering::Relaxed);
    }
}
