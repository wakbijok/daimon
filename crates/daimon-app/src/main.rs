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

    // ---- Phase 2b #19: assemble the action broker stack -----------------
    //
    // The broker is the single integration point between daimon-app
    // server-fns and the (vault + inventory + transport + audit) layer.
    // It is constructed once at boot from env-config and shared across all
    // request handlers via AppState.
    //
    // Env config:
    //   DAIMON_DATA_DIR        — directory holding vault.db, inventory.db,
    //                            audit.db. Default: ./daimon-data
    //   DAIMON_KNOWN_HOSTS_PATH — SSH known_hosts file. Default:
    //                            <data_dir>/known_hosts. Production should
    //                            point this at /var/lib/daimon/known_hosts.
    //   CREDENTIALS_DIRECTORY  — set by systemd. Production master-key path.
    //   DAIMON_MASTER_KEY_FILE — development fallback. WARNs loudly.
    let broker = match boot_broker().await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("daimon-app: failed to assemble broker stack: {e:#}");
            std::process::exit(1);
        }
    };

    // Init database
    let conn = db::init_db("daimon.db");

    // Ensure JWT secret exists
    let jwt_secret = match db::get_config(&conn, "jwt_secret") {
        Some(secret) => secret,
        None => {
            let secret = auth::generate_secret();
            db::set_config(&conn, "jwt_secret", &secret).unwrap();
            secret
        }
    };

    // Seed admin user if no users exist
    if db::find_user(&conn, "admin").is_none() {
        let password = std::env::var("DAIMON_ADMIN_PASSWORD")
            .unwrap_or_else(|_| {
                let pwd = auth::generate_secret();
                let short = &pwd[..16.min(pwd.len())];
                log!("Generated admin password: {}", short);
                short.to_string()
            });
        let hash = auth::hash_password(&password);
        db::create_user(&conn, "admin", &hash).unwrap();
        log!("Admin user created");
    }

    // Load clusters and build PVE clients
    let clusters = db::list_clusters(&conn);
    let mut pve_map = HashMap::new();
    for (cid, _name) in &clusters {
        if let Some((_id, _n, api_url, token, _notes, _created)) = db::get_cluster(&conn, cid) {
            let client = daimon_pve::Client::from_token_string(&api_url, &token);
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

    let app_state = AppState {
        db: Arc::new(tokio::sync::Mutex::new(conn)),
        jwt_secret,
        pve_clients: Arc::new(tokio::sync::RwLock::new(pve_map)),
        pve_cache: Arc::new(tokio::sync::RwLock::new(PveCache::new())),
        ws_broadcast: ws_tx,
        broker,
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
async fn boot_broker() -> anyhow::Result<std::sync::Arc<daimon_broker::Broker>> {
    use std::path::PathBuf;

    use anyhow::Context;
    use daimon_broker::production::{build_production_broker, BootConfig, MasterKeyHandle};

    let data_dir = PathBuf::from(
        std::env::var("DAIMON_DATA_DIR").unwrap_or_else(|_| "daimon-data".to_string()),
    );

    let known_hosts_path = PathBuf::from(
        std::env::var("DAIMON_KNOWN_HOSTS_PATH")
            .unwrap_or_else(|_| data_dir.join("known_hosts").to_string_lossy().into_owned()),
    );

    let master_key = MasterKeyHandle::from_systemd_or_dev_env().context(
        "load master key (set CREDENTIALS_DIRECTORY in systemd, or DAIMON_MASTER_KEY_FILE for dev)",
    )?;

    let broker = build_production_broker(BootConfig {
        data_dir,
        known_hosts_path,
        master_key,
    })
    .await
    .context("build_production_broker")?;

    Ok(broker)
}

#[cfg(not(feature = "ssr"))]
pub fn main() {}
