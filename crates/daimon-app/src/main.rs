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

    // Spawn background PVE polling task (30s interval)
    {
        let state = app_state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                let clients = state.pve_clients.read().await;
                let mut cache = state.pve_cache.write().await;
                for (cluster_id, client) in clients.iter() {
                    if let Ok(resources) = client.cluster_resources(None).await {
                        let old = cache.resources.get(cluster_id);
                        let new_json = serde_json::to_string(&resources).unwrap_or_default();
                        let changed = match old {
                            Some(old_res) => {
                                serde_json::to_string(old_res).unwrap_or_default() != new_json
                            }
                            None => true,
                        };
                        if changed {
                            cache.resources.insert(cluster_id.clone(), resources.clone());
                            let msg = WsServerMsg::Update {
                                scope: WsScope::ClusterResources {
                                    cluster_id: cluster_id.clone(),
                                },
                                data: serde_json::to_value(&resources).unwrap_or_default(),
                            };
                            let _ = state.ws_broadcast.send(
                                serde_json::to_string(&msg).unwrap_or_default(),
                            );
                        }
                        cache
                            .last_poll
                            .insert(cluster_id.clone(), std::time::Instant::now());
                    }
                }
            }
        });
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
