//! Phase 2b #19 proof-of-wire — minimal server-fn exercising the Broker
//! end-to-end from a Leptos request handler.
//!
//! `/admin/credentials`, `/admin/targets`, `/admin/audit` land in #12/#13/#14.
//! This module only proves the wire: the broker is reachable, admin gating
//! works, and a no-op admin call survives a round trip.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// Result of the broker-health probe. Counts whatever the broker can reach
/// from its `Arc<dyn Inventory>` handle. On a fresh boot this returns 0 of
/// each, which is enough to prove the wire — the call did not 401, did not
/// panic, did reach the broker, and did exercise an admin-proxy method.
///
/// Named `BrokerHealthReport` (not `BrokerHealth`) to avoid colliding with
/// the Leptos `#[server]` macro's auto-generated request struct, which is
/// derived in PascalCase from the function name `broker_health`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerHealthReport {
    pub target_count: usize,
    pub actor: String,
}

/// Admin-only health probe — proves the broker is wired into AppState.
///
/// Calls `Broker::list_targets(None)` (an agent-safe metadata listing) and
/// returns the count alongside the authenticated admin's username. The
/// implementation surface is intentionally small: subsequent admin pages
/// (#12/#13/#14) replace it with real CRUD + audit-query UIs.
#[server]
pub async fn broker_health() -> Result<BrokerHealthReport, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();

    let targets = state.broker.list_targets(None).await;
    Ok(BrokerHealthReport {
        target_count: targets.len(),
        actor: claims.sub,
    })
}
