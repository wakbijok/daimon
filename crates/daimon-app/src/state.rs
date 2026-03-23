#[cfg(feature = "ssr")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub jwt_secret: String,
}
