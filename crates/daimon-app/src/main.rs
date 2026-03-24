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

#[cfg(not(feature = "ssr"))]
pub fn main() {}
