#![recursion_limit = "512"]

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::Router;
    use leptos::logging::log;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use daimon_app::app::*;
    use daimon_app::db;
    use daimon_app::auth;
    use daimon_app::state::AppState;
    use std::sync::Arc;

    // ---- P3 commit 11 (AC-P3-06): install the tracing subscriber ONCE --------
    //
    // This is the FIRST thing main does — before any leptos `log!` — so every
    // broker/guard/transport/observer `#[instrument]`/`info!`/`warn!` span
    // (silently dropped today for lack of any subscriber) surfaces. JSON output
    // for machine ingestion; `RUST_LOG` overrides the default filter. Exactly
    // one `.init()` — a second global-default install would panic.
    {
        use tracing_subscriber::EnvFilter;
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "info,daimon=debug".into()),
            )
            .json()
            .init();
    }

    // ---- Phase 2b #19 + Phase 2c D3b: assemble the broker stack ----------
    //
    // The broker is the single integration point between daimon-app
    // server-fns and the (vault + inventory + transport + audit) layer.
    // Single-org: storage is PostgreSQL; boot needs DAIMON_PG_URL +
    // DAIMON_KNOWN_HOSTS_PATH.
    //
    // Env config:
    //   DAIMON_PG_URL           — postgres://... Default
    //                             postgres://$USER@localhost:5432/daimon
    //   DAIMON_KNOWN_HOSTS_PATH — SSH known_hosts file. Default
    //                             ./daimon-data/known_hosts.
    //   CREDENTIALS_DIRECTORY   — set by systemd. Production master-key path.
    //   DAIMON_MASTER_KEY_FILE  — development fallback. WARNs loudly.

    // Postgres pool + migrations first — the broker (and everything else)
    // expects the schema to exist. Migrations run on every boot so dev
    // iteration is one-shot — production runs them once via `daimon-migrate`.
    let pg_url = resolve_pg_url();
    let pool = match db::init_pool(&pg_url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("daimon-app: failed to initialise Postgres pool ({pg_url}): {e:#}");
            std::process::exit(1);
        }
    };

    let broker = match boot_broker().await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("daimon-app: failed to assemble broker stack: {e:#}");
            std::process::exit(1);
        }
    };

    // P6 — the config resolver (FR-CFG-02). Load the initial app_config snapshot
    // now that the pool is up. A load failure DEGRADES to an empty snapshot
    // (every read then falls to env/default) rather than failing boot — config
    // resolution must never be more fragile than the pre-P6 env-only path.
    let config = match daimon_app::config::ConfigResolver::load(&pool).await {
        Ok(c) => {
            log!("config resolver: loaded {} app_config key(s)", c.current().len());
            std::sync::Arc::new(c)
        }
        Err(e) => {
            eprintln!(
                "daimon-app: config resolver load failed ({e:#}) — degrading to env/default only"
            );
            std::sync::Arc::new(daimon_app::config::ConfigResolver::from_snapshot(
                daimon_app::config::ConfigSnapshot::default(),
            ))
        }
    };

    // Ensure JWT secret exists
    let jwt_secret = match db::get_config(&pool, "jwt_secret").await.unwrap_or(None) {
        Some(secret) => secret,
        None => {
            let secret = auth::generate_secret();
            db::set_config(&pool, "jwt_secret", &secret).await.unwrap();
            secret
        }
    };

    // Seed admin user if no users exist
    if db::find_user(&pool, "admin").await.unwrap_or(None).is_none() {
        let password = std::env::var("DAIMON_ADMIN_PASSWORD")
            .unwrap_or_else(|_| {
                let pwd = auth::generate_secret();
                let short = &pwd[..16.min(pwd.len())];
                log!("Generated admin password: {}", short);
                short.to_string()
            });
        let hash = auth::hash_password(&password);
        db::create_user(&pool, "admin", &hash, &["admin".to_string()])
            .await
            .unwrap();
        log!("Admin user created");
    }

    // Create broadcast channel for WebSocket updates
    let (ws_tx, _) = tokio::sync::broadcast::channel::<String>(256);

    let conf = get_configuration(None).unwrap();
    let addr = conf.leptos_options.site_addr;
    let leptos_options = conf.leptos_options;
    let routes = generate_route_list(App);

    // P2 — the multi-agent harness. Construct the in-process bus + capability
    // registry + supervisor (the FIRST production consumers of daimon-runtime),
    // spawn the RouterOS reference driver under supervision (which registers its
    // capabilities), and hold the assembled Harness in AppState. Chat +
    // orchestrator dispatch over this bus in P2 commits 5/6.
    let bus = daimon_runtime::InProcBus::new();
    let registry = daimon_runtime::CapabilityRegistry::new();
    let supervisor = Arc::new(daimon_runtime::Supervisor::new(bus.clone(), registry.clone()));
    // P3 commit 9 — capture a BusHandle for the observer BEFORE `bus` is moved
    // into `Harness::new` below. The observer emits `AnomalyDetected` onto this
    // handle (fire-and-forget); it holds only the abstract `Arc<dyn BusHandle>`,
    // never the concrete `InProcBus` (D21). InProcBus is Clone/broadcast-backed,
    // so this handle publishes to the SAME channel the supervised TriageAgent
    // subscribes on.
    let bus_handle = bus.handle();
    let routeros_driver = Arc::new(daimon_driver_firewall_routeros::RouterOsDriver::new(
        daimon_core::AgentId::new("agent:routeros"),
        broker.clone(),
        "agent:routeros",
    ));
    if let Err(e) = supervisor
        .spawn(routeros_driver.clone() as Arc<dyn daimon_core::Agent>)
        .await
    {
        eprintln!("daimon-app: failed to spawn RouterOS driver: {e:#}");
        std::process::exit(1);
    }

    // P2 commit 9 — the generic declarative ConnectorDriver (the SECOND driver).
    // Load `.toml` connector profiles from $DAIMON_CONNECTORS_DIR (default
    // deploy/connectors). Its capabilities register on the same bus + registry,
    // appear in the chat/planner catalogs, and are dispatchable by capability
    // exactly like the RouterOS driver — but over REST. If the dir is
    // absent/empty, skip gracefully (log).
    let connectors_dir = std::path::PathBuf::from(
        std::env::var("DAIMON_CONNECTORS_DIR").unwrap_or_else(|_| "deploy/connectors".to_string()),
    );
    match daimon_driver::ConnectorDriver::from_dir(
        "agent:connector",
        broker.clone(),
        &connectors_dir,
        "agent:connector",
    ) {
        Ok(drivers) if !drivers.is_empty() => {
            // One driver per target class (k8s→orchestrator, redfish→compute, …);
            // spawn each so heterogeneous connectors all register.
            let mut total_caps = 0usize;
            let driver_count = drivers.len();
            for driver in drivers {
                total_caps += driver.capabilities().len();
                let driver = Arc::new(driver);
                if let Err(e) = supervisor
                    .spawn(driver as Arc<dyn daimon_core::Agent>)
                    .await
                {
                    eprintln!("daimon-app: failed to spawn ConnectorDriver: {e:#}");
                    std::process::exit(1);
                }
            }
            log!(
                "connector drivers spawned from {} — {} capabilities across {} class-driver(s)",
                connectors_dir.display(),
                total_caps,
                driver_count
            );
        }
        Ok(_) => {
            log!(
                "no connector profiles at {} — skipping ConnectorDriver",
                connectors_dir.display()
            );
        }
        Err(e) => {
            eprintln!("daimon-app: failed to load connector profiles from {}: {e:#}", connectors_dir.display());
            std::process::exit(1);
        }
    }

    let harness = daimon_app::harness::Harness::new(bus, registry, supervisor.clone());
    log!("harness ready — drivers spawned under supervision");

    // P2 commit 8 — the REAL boot policy-coherence gate. Runs AFTER every driver
    // is spawned (registry populated), over the LIVE capability set: no write
    // capability may resolve to Allow under the shipped policy, and no
    // compensator may dangle. This is what the P1 hardcoded KNOWN_WRITE_CAPS
    // loop became once there was an actual fleet to lint.
    if let Err(e) = broker.lint_write_capabilities(&harness.capabilities().await) {
        eprintln!("daimon-app: boot policy-coherence check failed: {e}");
        std::process::exit(1);
    }
    log!("boot policy-coherence check passed — every write is deny/require_approval, no dangling compensators");
    // Phase 4 D4 — working memory tier. Redis when reachable; in-process
    // fallback otherwise. Set DAIMON_REDIS_URL=disabled to force in-process.
    // `from_url` builds only a LAZY pool (Ok even when Redis is down), so we
    // MUST ping() to know if Redis is really reachable — otherwise the fallback
    // never fires and every turn's conv load fails at request time.
    let working_memory: Arc<dyn daimon_redis::WorkingMemory> =
        match std::env::var("DAIMON_REDIS_URL") {
            Ok(s) if s == "disabled" => {
                log!("DAIMON_REDIS_URL=disabled — using in-process working memory");
                Arc::new(daimon_redis::InProcWorkingMemory::new())
            }
            Ok(url) => connect_working_memory(&url).await,
            Err(_) => connect_working_memory("redis://localhost:6379").await,
        };

    // Phase 8 — graph tier (NornicDB). Connect best-effort; if
    // DAIMON_GRAPH_URL isn't set or the daemon is unreachable, the
    // orchestrator + approvals UI fall back to no graph mirror /
    // no blast-radius summary respectively.
    let graph: Option<Arc<dyn daimon_graph::GraphClient>> = match std::env::var("DAIMON_GRAPH_URL")
    {
        Ok(uri) if !uri.is_empty() => match daimon_graph::NornicGraphClient::connect(
            &uri,
            std::env::var("DAIMON_GRAPH_USER").unwrap_or_default().as_str(),
            std::env::var("DAIMON_GRAPH_PASS").unwrap_or_default().as_str(),
        )
        .await
        {
            Ok(client) => {
                if let Err(e) = daimon_graph::ensure_schema(&client).await {
                    log!("graph schema bootstrap failed ({e}) — graph tier disabled");
                    None
                } else {
                    log!("connected to NornicDB graph tier at {uri}");
                    Some(Arc::new(client) as Arc<dyn daimon_graph::GraphClient>)
                }
            }
            Err(e) => {
                log!("graph connect to {uri} failed ({e}) — graph tier disabled");
                None
            }
        },
        _ => {
            log!("DAIMON_GRAPH_URL not set — graph tier disabled");
            None
        }
    };

    // P3 — long-term memory tier: the dmem SIDECAR client. Resolve the bearer
    // token from daimon's OWN vault (broker.vault_list_metadata → vault_reveal,
    // both audited), with a loud env fallback. Build the HTTP client against
    // DAIMON_DMEM_URL (default http://localhost:7071). On any init failure use
    // NullMemory — a missing/misconfigured sidecar degrades chat recall + admin
    // memory, it never fails boot.
    let memory: Arc<dyn daimon_memory::MemoryService> = {
        let dmem_url = std::env::var("DAIMON_DMEM_URL")
            .unwrap_or_else(|_| "http://localhost:7071".to_string());
        match resolve_dmem_token(&broker).await {
            Some(token) => match daimon_memory::DmemHttpMemory::new(&dmem_url, token) {
                Ok(client) => {
                    log!("memory tier: dmem sidecar client at {}", dmem_url);
                    Arc::new(client) as Arc<dyn daimon_memory::MemoryService>
                }
                Err(e) => {
                    log!(
                        "memory tier: dmem client init failed ({e}) — using NullMemory (recall degrades)"
                    );
                    Arc::new(daimon_memory::NullMemory)
                }
            },
            None => {
                log!(
                    "memory tier: no dmem bearer token resolved (vault entry 'dmem-bearer' \
                     missing and DAIMON_DMEM_TOKEN unset) — using NullMemory (recall degrades)"
                );
                Arc::new(daimon_memory::NullMemory)
            }
        }
    };

    // P2 commit 6 — the orchestrator dispatches steps over the SAME bus+registry
    // as the Harness (via a shared `Dispatcher`), NOT the broker directly. This
    // preserves D21 (the orchestrator never imports vault/transport) and routes
    // plan writes through the identical driver → broker → Guard path as chat.
    //
    // P3 commit 10 — `.with_memory(memory.clone())` so a terminal plan state
    // (Succeeded → Decision, Failed → Incident) captures a typed record AFTER
    // the status/audit write (fail-soft). `memory` was built above.
    let orchestrator_service = daimon_orchestrator::OrchestratorService::new(
        pool.clone(),
        harness.dispatcher().clone(),
    )
    .with_memory(memory.clone());
    let orchestrator = Arc::new(match graph.clone() {
        Some(g) => orchestrator_service.with_graph(g),
        None => orchestrator_service,
    });

    // P3 commits 8+9 — spawn the TriageAgent under the SAME Supervisor as the
    // drivers. This is the LOAD-BEARING routing fact: the observer emits an
    // `AnomalyDetected` envelope `ByCapability` `"harness.triage.anomaly"`, and
    // a ByCapability envelope reaches an agent ONLY if it is registered under
    // the Supervisor advertising that capability. The TriageAgent advertises
    // exactly that one capability, so the anomaly routes to it, where it opens a
    // PERSISTED-but-NOT-RUN triage plan (remediation stays behind run_plan's
    // guard+approval — triage never calls run_plan). It holds the orchestrator +
    // a Dispatcher (the same bus+registry as the Harness) + memory.
    {
        let triage = daimon_triage::TriageAgent::new(
            orchestrator.clone(),
            harness.dispatcher().clone(),
            memory.clone(),
        );
        if let Err(e) = supervisor
            .spawn(Arc::new(triage) as Arc<dyn daimon_core::Agent>)
            .await
        {
            eprintln!("daimon-app: failed to spawn TriageAgent: {e:#}");
            std::process::exit(1);
        }
        log!("triage agent spawned under supervision (harness.triage.anomaly)");
    }

    // P3 commit 11 (AC-P3-06) — daimon's OWN self-metrics. Hand-rolled AtomicU64
    // counters rendered as Prometheus text by /metrics (no prometheus/protobuf
    // dep — musl-size). The three observer-owned counters are shared
    // Arc<AtomicU64> handles passed into ObserverIngest below, so /metrics reads
    // the SAME atomics the observer increments — one source of truth, no
    // observer→app dependency.
    let self_metrics = Arc::new(daimon_app::observability::SelfMetrics::new());

    // P4-7 — build the messaging-gateway registry from the channels.* config +
    // the vault-held bot secrets. Done before AppState moves `broker`/`pool`.
    // Webhook adapters land in the registry; pollers (Matrix /sync, Telegram
    // getUpdates) are returned to spawn once AppState exists.
    let (gateway_registry, gateway_pollers) = build_gateways(&broker, &pool).await;

    // P5-5 — load skills (workflow-templates) from $DAIMON_SKILLS_DIR
    // (default deploy/skills). A malformed skill fails boot loudly.
    let skills_dir = std::path::PathBuf::from(
        std::env::var("DAIMON_SKILLS_DIR").unwrap_or_else(|_| "deploy/skills".to_string()),
    );
    let skills = match daimon_app::skills::SkillLibrary::from_dir(&skills_dir) {
        Ok(s) => {
            log!("loaded {} skill(s) from {}", s.len(), skills_dir.display());
            std::sync::Arc::new(s)
        }
        Err(e) => {
            eprintln!("daimon-app: failed to load skills from {}: {e}", skills_dir.display());
            std::process::exit(1);
        }
    };

    let app_state = AppState {
        db: pool,
        jwt_secret,
        ws_broadcast: ws_tx,
        broker,
        harness,
        working_memory,
        orchestrator,
        graph,
        memory,
        self_metrics: self_metrics.clone(),
        // P4-7: the gateway registry, built from the channels.* config —
        // webhook adapters (Telegram) registered here; the Matrix poller is
        // spawned just below once AppState exists. Empty when nothing is enabled.
        gateways: std::sync::Arc::new(gateway_registry),
        skills,
        config,
    };

    // P4-7 (FR-GW-05): spawn each enabled poller (Matrix /sync, Telegram
    // getUpdates). Pollers need the full AppState (for the shared inbound
    // pipeline), so they are spawned after the struct.
    for poller in gateway_pollers {
        daimon_app::gw::spawn_poller(app_state.clone(), poller);
    }

    // Phase 7 — observer ingest. Only spawns if DAIMON_PROM_URL is set.
    //
    // Phase 8 lock: metric streams land in VictoriaMetrics
    // (`DAIMON_VM_URL`, default http://localhost:8428). The injected
    // sink is a VictoriaMetricsSink; the Postgres observer.metrics table
    // was dropped in V015.
    if let Ok(prom_url) = std::env::var("DAIMON_PROM_URL") {
        use daimon_observer::{
            NamedQueryLibrary, ObserverIngest, ObserverIngestConfig, VictoriaMetricsSink,
        };
        let vm_url = std::env::var("DAIMON_VM_URL")
            .unwrap_or_else(|_| "http://localhost:8428".to_string());
        let sink = std::sync::Arc::new(VictoriaMetricsSink::new(vm_url));
        match ObserverIngest::new(
            ObserverIngestConfig {
                prom_url: prom_url.clone(),
                interval: std::time::Duration::from_secs(30),
            },
            sink,
            app_state.db.clone(),
            NamedQueryLibrary::default_library(),
        ) {
            Ok(ingest) => {
                // P3 commit 9 — wire the bus so a persisted anomaly also emits
                // an `AnomalyDetected` envelope for the TriageAgent
                // (fire-and-forget; zero-subscriber send is a no-op, so
                // persistence is never blocked).
                //
                // P3 commit 11 — also hand the observer the three shared
                // self-metric counter handles (ingest cycles / anomalies raised
                // / sink push failures). It increments them via std::sync::atomic
                // alone, so daimon-observer gains NO dependency on the app's
                // SelfMetrics type; /metrics still renders from these same
                // atomics.
                let (m_ingest, m_anomalies, m_failures) = self_metrics.observer_handles();
                log!("observer ingest spawned against {} (bus-wired for triage, self-metrics wired)", prom_url);
                ingest
                    .with_bus(bus_handle.clone())
                    .with_metrics(m_ingest, m_anomalies, m_failures)
                    .spawn();
            }
            Err(e) => {
                log!("observer ingest init failed ({}) — skipping", e);
            }
        }
    } else {
        log!("DAIMON_PROM_URL not set — observer Prometheus ingest disabled");
    }

    // Build router: WS route first (needs Extension), then Leptos routes.
    //
    // P3 commit 11 (AC-P3-06) — the self-observability routes mount here too,
    // BEFORE `.leptos_routes_with_context`, exactly like `/api/v1/ws`. They are
    // UNAUTHENTICATED on purpose: `/healthz` + `/metrics` are an infra surface
    // meant to sit behind the reverse proxy / systemd probe, distinct from the
    // authed `/api/v1/ws` (which reaches the LLM + SSH dispatch). Both read from
    // the same `AppState` Extension applied by the `.layer` below.
    let app = Router::new()
        .route(
            "/api/v1/ws",
            axum::routing::get(daimon_app::ws::ws_handler),
        )
        // P4 (FR-GW-05/07): the inbound webhook route for every webhook adapter.
        // Registered here (before leptos_routes) exactly like `/api/v1/ws`. It is
        // internet-facing and UNauthenticated at the HTTP layer on purpose —
        // authenticity is the per-request signature/secret the adapter verifies
        // (FR-GW-07), then identity is bound fail-closed (FR-GW-08). An unknown
        // or disabled channel 404s (no adapter registered).
        .route(
            "/api/v1/gw/{channel}",
            axum::routing::post(daimon_app::gw::gateway_webhook),
        )
        .route(
            "/healthz",
            axum::routing::get(daimon_app::observability::healthz),
        )
        .route(
            "/metrics",
            axum::routing::get(daimon_app::observability::metrics),
        )
        .layer(axum::Extension(app_state.clone()))
        .leptos_routes_with_context(
            &leptos_options,
            routes,
            {
                let app_state = app_state.clone();
                move || {
                    leptos::context::provide_context(app_state.clone());
                }
            },
            {
                let leptos_options = leptos_options.clone();
                move || shell(leptos_options.clone())
            },
        )
        .fallback(leptos_axum::file_and_error_handler(shell))
        .with_state(leptos_options);

    log!("daimon listening on http://{}", &addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

/// Assemble the production broker stack at boot.
///
/// All filesystem paths come from env config (see `main` for the contract).
/// The master key is loaded via `MasterKey::from_systemd_or_dev_env` — systemd
/// `LoadCredentialEncrypted` in production, `DAIMON_MASTER_KEY_FILE` for local
/// dev (with a loud WARN log).
///
/// Per D21, daimon-app does NOT import `daimon-vault`, `daimon-inventory`,
/// or `daimon-transport` directly. The assembly happens inside
/// `daimon_broker::production::build_production_broker`, which is the only
/// path the spec permits for a long-running I/O adapter.
#[cfg(feature = "ssr")]
async fn boot_broker() -> anyhow::Result<std::sync::Arc<daimon_broker::Broker>> {
    use std::path::PathBuf;

    use anyhow::Context;
    use daimon_broker::production::{build_production_broker, BootConfig, MasterKeyHandle};

    let pg_url = resolve_pg_url();

    let data_dir = std::env::var("DAIMON_DATA_DIR").unwrap_or_else(|_| "daimon-data".to_string());
    let known_hosts_path = PathBuf::from(
        std::env::var("DAIMON_KNOWN_HOSTS_PATH")
            .unwrap_or_else(|_| format!("{data_dir}/known_hosts")),
    );
    let kill_path = PathBuf::from(
        std::env::var("DAIMON_KILL_PATH").unwrap_or_else(|_| format!("{data_dir}/KILL")),
    );
    let policy_path = PathBuf::from(
        std::env::var("DAIMON_POLICY_PATH").unwrap_or_else(|_| format!("{data_dir}/policy.toml")),
    );

    let master_key = MasterKeyHandle::from_systemd_or_dev_env().context(
        "load master key (set CREDENTIALS_DIRECTORY in systemd, or DAIMON_MASTER_KEY_FILE for dev)",
    )?;

    let broker = build_production_broker(BootConfig {
        pg_url,
        known_hosts_path,
        master_key,
        kill_path,
        policy_path,
    })
    .await
    .context("build_production_broker")?;

    Ok(broker)
}

/// Resolve the dmem sidecar bearer token from daimon's OWN vault.
///
/// Preferred path: `broker.vault_list_metadata` (audited) → find the credential
/// named `dmem-bearer` → `broker.vault_reveal` (audited) → its `ApiToken.token`.
/// Both broker calls emit audit events, so the token resolution is attributable.
/// The synthetic actor `"system:boot"` marks these as boot-time reads.
///
/// Fallback: the `DAIMON_DMEM_TOKEN` env var, with a loud WARN — an env token is
/// not audited the way a vault reveal is, so it is a dev/bootstrap convenience,
/// not the production path.
///
/// Returns `None` when neither source yields a token; the caller then uses
/// `NullMemory` (memory degrades, boot proceeds).
#[cfg(feature = "ssr")]
async fn resolve_dmem_token(broker: &std::sync::Arc<daimon_broker::Broker>) -> Option<String> {
    use leptos::logging::log;
    const CRED_NAME: &str = "dmem-bearer";

    if let Some(t) = resolve_vault_api_token(broker, CRED_NAME).await {
        return Some(t);
    }
    match std::env::var("DAIMON_DMEM_TOKEN") {
        Ok(t) if !t.is_empty() => {
            log!(
                "resolve_dmem_token: WARNING using DAIMON_DMEM_TOKEN env fallback — the dmem \
                 bearer should live in daimon's vault as credential '{CRED_NAME}' (audited reveal)"
            );
            Some(t)
        }
        _ => None,
    }
}

/// Resolve a named `ApiToken` credential from daimon's OWN vault (audited reveal
/// under the synthetic `"system:boot"` actor). Shared by the dmem bearer and the
/// gateway bot tokens / signing secrets (FR-GW-17: secrets by reference, never
/// plaintext config). Returns `None` (with a log) on any miss so the caller can
/// degrade — a gateway simply stays disabled.
#[cfg(feature = "ssr")]
async fn resolve_vault_api_token(
    broker: &std::sync::Arc<daimon_broker::Broker>,
    cred_name: &str,
) -> Option<String> {
    use daimon_broker::Credential;
    use leptos::logging::log;
    const ACTOR: &str = "system:boot";

    match broker.vault_list_metadata(ACTOR).await {
        Ok(metas) => {
            if let Some(meta) = metas.into_iter().find(|m| m.name == cred_name) {
                match broker.vault_reveal(ACTOR, meta.id).await {
                    // `Credential` is ZeroizeOnDrop — match by ref, clone the secret.
                    Ok(cred) => match &cred {
                        Credential::ApiToken { token } => return Some(token.clone()),
                        other => log!(
                            "resolve_vault_api_token: '{cred_name}' is not an ApiToken (kind={:?})",
                            other.kind()
                        ),
                    },
                    Err(e) => log!("resolve_vault_api_token: reveal('{cred_name}') failed ({e})"),
                }
            } else {
                log!("resolve_vault_api_token: credential '{cred_name}' not found in vault");
            }
        }
        Err(e) => log!("resolve_vault_api_token: vault_list_metadata failed ({e})"),
    }
    None
}

/// Build the gateway registry from the `channels.*` config (P4-7, FR-GW-05/16).
///
/// A channel is wired only if `channels.<ch>.enabled` is truthy AND its
/// vault-held secret(s) resolve — otherwise it is skipped with a log (fail-safe:
/// a mis-configured channel never blocks boot). Returns the webhook registry
/// (Telegram) plus an optional Matrix poller adapter for the caller to spawn
/// once `AppState` exists. With nothing enabled, the registry is empty and no
/// poller runs — the `/api/v1/gw/*` route then 404s (no footprint).
#[cfg(feature = "ssr")]
async fn build_gateways(
    broker: &std::sync::Arc<daimon_broker::Broker>,
    pool: &daimon_db::Pool,
) -> (
    daimon_app::gw::GatewayRegistry,
    Vec<std::sync::Arc<dyn daimon_gateway::PollingGateway>>,
) {
    use daimon_app::gw::{AppConfigCursor, GatewayRegistry};
    use daimon_gateway::PollingGateway;
    use daimon_gateway::adapters::{
        matrix::MatrixAdapter,
        telegram::{TelegramAdapter, TelegramPollAdapter},
    };
    use leptos::logging::log;
    use std::sync::Arc;

    async fn truthy(pool: &daimon_db::Pool, key: &str) -> bool {
        match daimon_app::db::get_config_json(pool, key).await {
            Ok(Some(serde_json::Value::Bool(b))) => b,
            Ok(Some(serde_json::Value::String(s))) => {
                matches!(s.to_lowercase().as_str(), "true" | "1" | "yes" | "on")
            }
            _ => false,
        }
    }
    async fn cfg_str(pool: &daimon_db::Pool, key: &str) -> Option<String> {
        match daimon_app::db::get_config_json(pool, key).await {
            Ok(Some(serde_json::Value::String(s))) if !s.trim().is_empty() => Some(s),
            _ => None,
        }
    }

    let mut registry = GatewayRegistry::new();
    let mut pollers: Vec<Arc<dyn PollingGateway>> = Vec::new();

    // --- Telegram — poll (default) or webhook ---
    if truthy(pool, "channels.telegram.enabled").await {
        let mode = cfg_str(pool, "channels.telegram.mode")
            .await
            .unwrap_or_else(|| "poll".to_string());
        // Bot token: vault credential (by name) OR the DAIMON_GW_TELEGRAM_TOKEN
        // dev env fallback (loud — production should use the vault, FR-GW-17).
        let token = match cfg_str(pool, "channels.telegram.bot_token_cred").await {
            Some(tc) => resolve_vault_api_token(broker, &tc).await,
            None => None,
        }
        .or_else(|| {
            std::env::var("DAIMON_GW_TELEGRAM_TOKEN")
                .ok()
                .filter(|s| !s.is_empty())
                .inspect(|_| {
                    log!(
                        "gateway: telegram using DAIMON_GW_TELEGRAM_TOKEN env fallback (dev) — production should hold the bot token in the vault (FR-GW-17)"
                    );
                })
        });
        match token {
            Some(token) => {
                if mode.eq_ignore_ascii_case("webhook") {
                    let secret = match cfg_str(pool, "channels.telegram.webhook_secret_cred").await {
                        Some(sc) => resolve_vault_api_token(broker, &sc).await,
                        None => None,
                    }
                    .or_else(|| {
                        std::env::var("DAIMON_GW_TELEGRAM_WEBHOOK_SECRET")
                            .ok()
                            .filter(|s| !s.is_empty())
                    });
                    match secret {
                        Some(secret) => {
                            registry.register(Arc::new(TelegramAdapter::new(token, secret)));
                            log!("gateway: telegram ENABLED (webhook /api/v1/gw/telegram)");
                        }
                        None => log!(
                            "gateway: telegram mode=webhook but no webhook secret (vault cred + DAIMON_GW_TELEGRAM_WEBHOOK_SECRET both unset) — skipping"
                        ),
                    }
                } else {
                    // Poll (default) — getUpdates, no ingress. Reuses an internal
                    // bot (e.g. John's) with no public endpoint.
                    let cursor =
                        Arc::new(AppConfigCursor::new(pool.clone(), "channels.telegram.offset"));
                    pollers.push(Arc::new(TelegramPollAdapter::new(token, cursor)));
                    log!("gateway: telegram ENABLED (getUpdates poller)");
                }
            }
            None => log!(
                "gateway: telegram enabled but no bot token (channels.telegram.bot_token_cred + DAIMON_GW_TELEGRAM_TOKEN both unset) — skipping"
            ),
        }
    }

    // --- Matrix (/sync poller) ---
    if truthy(pool, "channels.matrix.enabled").await {
        let homeserver = cfg_str(pool, "channels.matrix.homeserver").await;
        let token_cred = cfg_str(pool, "channels.matrix.access_token_cred").await;
        match (homeserver, token_cred) {
            (Some(hs), Some(tc)) => match resolve_vault_api_token(broker, &tc).await {
                Some(token) => {
                    let cursor =
                        Arc::new(AppConfigCursor::new(pool.clone(), "channels.matrix.since"));
                    pollers.push(Arc::new(MatrixAdapter::new(hs, token, cursor)));
                    log!("gateway: matrix ENABLED (/sync poller)");
                }
                None => log!(
                    "gateway: matrix enabled but access-token credential did not resolve — skipping"
                ),
            },
            _ => log!(
                "gateway: matrix enabled but channels.matrix.{{homeserver,access_token_cred}} unset — skipping"
            ),
        }
    }

    (registry, pollers)
}

/// Build the working-memory tier: Redis if it PINGs, else in-process. The ping
/// is load-bearing — `RedisWorkingMemory::from_url` only lazily configures a
/// pool and returns Ok even against a dead Redis (deadpool connects on first
/// use), so without an active ping the in-process fallback would never trigger
/// and the first chat turn would fail with connection-refused.
#[cfg(feature = "ssr")]
async fn connect_working_memory(url: &str) -> std::sync::Arc<dyn daimon_redis::WorkingMemory> {
    use leptos::logging::log;
    use std::sync::Arc;
    match daimon_redis::RedisWorkingMemory::from_url(url) {
        Ok(c) => match c.ping().await {
            Ok(()) => {
                log!("connected to Redis at {url}");
                Arc::new(c)
            }
            Err(e) => {
                log!("Redis at {url} unreachable ({e}) — using in-process working memory");
                Arc::new(daimon_redis::InProcWorkingMemory::new())
            }
        },
        Err(e) => {
            log!("Redis config for {url} failed ({e}) — using in-process working memory");
            Arc::new(daimon_redis::InProcWorkingMemory::new())
        }
    }
}

#[cfg(feature = "ssr")]
fn resolve_pg_url() -> String {
    if let Ok(u) = std::env::var("DAIMON_PG_URL") {
        return u;
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "postgres".into());
    format!("postgres://{user}@localhost:5432/daimon")
}

#[cfg(not(feature = "ssr"))]
pub fn main() {}
