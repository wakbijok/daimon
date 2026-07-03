#![recursion_limit = "512"]

pub mod app;
pub mod components;
pub mod pages;
pub mod db;
pub mod auth;
pub mod auth_guard;
pub mod state;
#[cfg(feature = "ssr")]
pub mod harness;
pub mod ws;
#[cfg(feature = "ssr")]
pub mod observability;
pub mod admin_approvals;
pub mod admin_audit;
pub mod admin_broker;
pub mod admin_chat_sessions;
pub mod admin_credentials;
pub mod admin_gateways;
pub mod admin_graph;
pub mod admin_memory;
pub mod admin_observer;
pub mod admin_plans;
pub mod admin_settings;
pub mod admin_targets;
pub mod iam;
#[cfg(feature = "ssr")]
pub mod chat;
#[cfg(feature = "ssr")]
pub mod gw;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use app::*;
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
}
