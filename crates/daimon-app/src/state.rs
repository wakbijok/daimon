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
}
