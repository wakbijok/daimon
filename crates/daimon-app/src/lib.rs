#![recursion_limit = "512"]

pub mod app;
pub mod components;
pub mod pages;
pub mod db;
pub mod auth;
pub mod auth_guard;
pub mod state;
pub mod ws;
pub mod admin_broker;
pub mod admin_credentials;
pub mod admin_targets;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use app::*;
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
}
