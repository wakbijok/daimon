#[cfg(feature = "ssr")]
use std::sync::Arc;
#[cfg(feature = "ssr")]
use std::collections::HashMap;

/// Cached PVE data — populated by background poller, read by WS handler
#[cfg(feature = "ssr")]
pub struct PveCache {
    pub resources: HashMap<String, Vec<daimon_pve::PveResource>>,
    pub node_rrd: HashMap<(String, String), Vec<daimon_pve::RrdDataPoint>>,
    pub last_poll: HashMap<String, std::time::Instant>,
}

#[cfg(feature = "ssr")]
impl PveCache {
    pub fn new() -> Self {
        Self {
            resources: HashMap::new(),
            node_rrd: HashMap::new(),
            last_poll: HashMap::new(),
        }
    }
}

#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct AppState {
    /// Phase 2c D3b: Postgres pool. Replaces the prior
    /// `Arc<Mutex<rusqlite::Connection>>`.
    pub db: daimon_db::Pool,
    /// Tenant scope this AppState is constructed against. D6 will move this
    /// to per-request resolution from the JWT tenant claim.
    pub tenant_id: uuid::Uuid,
    pub jwt_secret: String,
    pub pve_clients: Arc<tokio::sync::RwLock<HashMap<String, daimon_pve::Client>>>,
    pub pve_cache: Arc<tokio::sync::RwLock<PveCache>>,
    pub ws_broadcast: tokio::sync::broadcast::Sender<String>,
    /// Phase 2b #19: action broker — the single integration point between
    /// daimon-app server-fns and the D19/D20/D22/D23/D24 stack
    /// (vault + inventory + transport + audit). Constructed once at boot
    /// in `main.rs::boot_broker()` and shared across all request handlers.
    pub broker: Arc<daimon_broker::Broker>,
    /// Phase 4 D2 — first worker agent. Owned here so the chat handler
    /// (Phase 4 D3) can dispatch tool calls without spinning up a fresh
    /// agent per request.
    pub network_agent: Arc<daimon_tool_network::NetworkAgent>,
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
