#[cfg(feature = "ssr")]
use std::sync::Arc;
#[cfg(feature = "ssr")]
use std::collections::HashMap;

#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<tokio::sync::Mutex<rusqlite::Connection>>,
    pub jwt_secret: String,
    pub pve_clients: Arc<tokio::sync::RwLock<HashMap<String, daimon_pve::Client>>>,
}
