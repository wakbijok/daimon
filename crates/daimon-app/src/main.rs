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
    use daimon_app::state::{AppState, PveCache};
    use daimon_app::ws::WsServerMsg;
    use daimon_app::ws::WsScope;
    use std::sync::Arc;
    use std::collections::HashMap;

    // ---- Phase 2b #19 + Phase 2c D3b: assemble the broker stack ----------
    //
    // The broker is the single integration point between daimon-app
    // server-fns and the (vault + inventory + transport + audit) layer.
    // Phase 2c moved storage from SQLite files to PostgreSQL; the boot now
    // needs DAIMON_PG_URL + DAIMON_TENANT_SLUG + DAIMON_KNOWN_HOSTS_PATH.
    //
    // Env config:
    //   DAIMON_PG_URL           — postgres://... Default
    //                             postgres://$USER@localhost:5432/daimon
    //   DAIMON_TENANT_SLUG      — tenant scope. Default `default`.
    //   DAIMON_KNOWN_HOSTS_PATH — SSH known_hosts file. Default
    //                             ./daimon-data/known_hosts.
    //   CREDENTIALS_DIRECTORY   — set by systemd. Production master-key path.
    //   DAIMON_MASTER_KEY_FILE  — development fallback. WARNs loudly.
    let tenant_slug = std::env::var("DAIMON_TENANT_SLUG").unwrap_or_else(|_| "default".into());

    let broker = match boot_broker(&tenant_slug).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("daimon-app: failed to assemble broker stack: {e:#}");
            std::process::exit(1);
        }
    };

    // Phase 2c D3b: Postgres pool replaces SQLite. Migrations run on every
    // boot so dev iteration is one-shot — production runs them once via
    // `daimon-migrate`.
    let pg_url = resolve_pg_url();
    let pool = match db::init_pool(&pg_url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("daimon-app: failed to initialise Postgres pool ({pg_url}): {e:#}");
            std::process::exit(1);
        }
    };
    let tenant_id = match db::resolve_tenant_id(&pool, &tenant_slug).await {
        Ok(id) => id,
        Err(e) => {
            eprintln!("daimon-app: tenant `{tenant_slug}` not found: {e:#}");
            std::process::exit(1);
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
        db::create_user(&pool, tenant_id, "admin", &hash).await.unwrap();
        log!("Admin user created");
    }

    // Load clusters and build PVE clients
    let clusters = db::list_clusters(&pool, tenant_id).await.unwrap_or_default();
    let mut pve_map = HashMap::new();
    for (cid, _name) in &clusters {
        if let Some(c) = db::get_cluster(&pool, tenant_id, cid).await.unwrap_or(None) {
            let client = daimon_pve::Client::from_token_string(&c.api_url, &c.token);
            pve_map.insert(cid.clone(), client);
        }
    }
    log!("Loaded {} PVE cluster(s)", pve_map.len());

    // Create broadcast channel for WebSocket updates
    let (ws_tx, _) = tokio::sync::broadcast::channel::<String>(256);

    let conf = get_configuration(None).unwrap();
    let addr = conf.leptos_options.site_addr;
    let leptos_options = conf.leptos_options;
    let routes = generate_route_list(App);

    // Phase 4 D2 — first worker agent. Held in AppState so the chat
    // handler can dispatch tool calls without recreating per request.
    let network_agent = Arc::new(daimon_tool_network::NetworkAgent::new(
        daimon_core::AgentId::new("network"),
        broker.clone(),
        "agent:network",
    ));
    // Phase 4 D4 — working memory tier. Redis when reachable; in-process
    // fallback otherwise. Set DAIMON_REDIS_URL=disabled to force in-process.
    let working_memory: Arc<dyn daimon_redis::WorkingMemory> = match std::env::var("DAIMON_REDIS_URL") {
        Ok(s) if s == "disabled" => {
            log!("DAIMON_REDIS_URL=disabled — using in-process working memory");
            Arc::new(daimon_redis::InProcWorkingMemory::new())
        }
        Ok(url) => match daimon_redis::RedisWorkingMemory::from_url(&url) {
            Ok(c) => {
                log!("connected to Redis at {}", url);
                Arc::new(c)
            }
            Err(e) => {
                log!("Redis connect failed ({e}) — falling back to in-process working memory");
                Arc::new(daimon_redis::InProcWorkingMemory::new())
            }
        },
        Err(_) => match daimon_redis::RedisWorkingMemory::from_url("redis://localhost:6379") {
            Ok(c) => {
                log!("connected to Redis at redis://localhost:6379 (default)");
                Arc::new(c)
            }
            Err(e) => {
                log!("Redis default-connect failed ({e}) — using in-process working memory");
                Arc::new(daimon_redis::InProcWorkingMemory::new())
            }
        },
    };

    let orchestrator = Arc::new(daimon_orchestrator::OrchestratorService::new(
        pool.clone(),
        broker.clone(),
    ));

    let app_state = AppState {
        db: pool,
        tenant_id,
        jwt_secret,
        pve_clients: Arc::new(tokio::sync::RwLock::new(pve_map)),
        pve_cache: Arc::new(tokio::sync::RwLock::new(PveCache::new())),
        ws_broadcast: ws_tx,
        broker,
        network_agent,
        working_memory,
        orchestrator,
    };

    // Phase 7 — PlatformPoller per cluster + push metrics to observer.metrics.
    // The poller calls Platform::list_workloads on a fixed interval and
    // broadcasts the snapshot back through ws_broadcast in the existing
    // WsServerMsg::Update shape. Metric points (cpu/mem/disk per workload)
    // land in observer.metrics for the time-series tier.
    {
        let state = app_state.clone();
        let pollers_handle = tokio::spawn(async move {
            use std::time::Duration;
            use daimon_observer::{MetricPoint, MetricSink, PostgresMetricSink};

            let clients = state.pve_clients.read().await.clone();
            let drivers: Vec<(String, std::sync::Arc<daimon_tool_platform::PveDriver>)> = clients
                .into_iter()
                .map(|(id, c)| (id.clone(), std::sync::Arc::new(daimon_tool_platform::PveDriver::new(id, c))))
                .collect();
            let metric_sink = std::sync::Arc::new(PostgresMetricSink::new(state.db.clone()));

            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                for (cluster_id, driver) in &drivers {
                    let now = chrono::Utc::now();
                    let driver_dyn: std::sync::Arc<dyn daimon_tool_platform::Platform> = driver.clone();
                    let workloads = match driver_dyn.list_workloads().await {
                        Ok(w) => w,
                        Err(e) => {
                            tracing::warn!(cluster = %cluster_id, error = %e, "platform list_workloads failed");
                            continue;
                        }
                    };

                    // Convert to legacy PveResource for the existing pve_cache +
                    // WsServerMsg::Update path (no UI changes needed).
                    if let Ok(resources) = driver.client().cluster_resources(None).await {
                        let mut cache = state.pve_cache.write().await;
                        let changed = cache
                            .resources
                            .get(cluster_id)
                            .map(|prev| serde_json::to_string(prev).unwrap_or_default()
                                != serde_json::to_string(&resources).unwrap_or_default())
                            .unwrap_or(true);
                        if changed {
                            cache.resources.insert(cluster_id.clone(), resources.clone());
                            let msg = WsServerMsg::Update {
                                scope: WsScope::ClusterResources { cluster_id: cluster_id.clone() },
                                data: serde_json::to_value(&resources).unwrap_or_default(),
                            };
                            let _ = state.ws_broadcast.send(serde_json::to_string(&msg).unwrap_or_default());
                        }
                        cache.last_poll.insert(cluster_id.clone(), std::time::Instant::now());
                    }

                    // Push per-workload metric points into observer.metrics.
                    let points: Vec<MetricPoint> = workloads
                        .iter()
                        .flat_map(|w| {
                            let labels = serde_json::json!({
                                "workload_id": w.id,
                                "workload_name": w.name,
                                "node": w.node,
                                "kind": w.kind,
                                "status": w.status,
                            });
                            vec![
                                MetricPoint {
                                    ts: now,
                                    source: "pve".into(),
                                    source_id: cluster_id.clone(),
                                    name: "pve.workload.cpu_pct".into(),
                                    value: w.cpu_pct as f64,
                                    labels: labels.clone(),
                                },
                                MetricPoint {
                                    ts: now,
                                    source: "pve".into(),
                                    source_id: cluster_id.clone(),
                                    name: "pve.workload.mem_used_bytes".into(),
                                    value: w.mem_used as f64,
                                    labels: labels.clone(),
                                },
                                MetricPoint {
                                    ts: now,
                                    source: "pve".into(),
                                    source_id: cluster_id.clone(),
                                    name: "pve.workload.disk_used_bytes".into(),
                                    value: w.disk_used as f64,
                                    labels,
                                },
                            ]
                        })
                        .collect();
                    if let Err(e) = metric_sink.push_batch(state.tenant_id, points).await {
                        tracing::warn!(error = %e, "observer metrics push failed");
                    }
                }
            }
        });
        let _ = pollers_handle; // we don't await the loop — it owns its own runtime
    }

    // Phase 7 — observer ingest. Only spawns if DAIMON_PROM_URL is set.
    if let Ok(prom_url) = std::env::var("DAIMON_PROM_URL") {
        use daimon_observer::{NamedQueryLibrary, ObserverIngest, ObserverIngestConfig};
        match ObserverIngest::new(
            ObserverIngestConfig {
                tenant_id: app_state.tenant_id,
                prom_url: prom_url.clone(),
                interval: std::time::Duration::from_secs(30),
            },
            app_state.db.clone(),
            NamedQueryLibrary::default_library(),
        ) {
            Ok(ingest) => {
                log!("observer ingest spawned against {}", prom_url);
                ingest.spawn();
            }
            Err(e) => {
                log!("observer ingest init failed ({}) — skipping", e);
            }
        }
    } else {
        log!("DAIMON_PROM_URL not set — observer Prometheus ingest disabled");
    }

    // Build router: WS route first (needs Extension), then Leptos routes
    let app = Router::new()
        .route(
            "/api/v1/ws",
            axum::routing::get(daimon_app::ws::ws_handler),
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
async fn boot_broker(tenant_slug: &str) -> anyhow::Result<std::sync::Arc<daimon_broker::Broker>> {
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
        tenant_slug: tenant_slug.to_string(),
        known_hosts_path,
        master_key,
        kill_path,
        policy_path,
    })
    .await
    .context("build_production_broker")?;

    Ok(broker)
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
