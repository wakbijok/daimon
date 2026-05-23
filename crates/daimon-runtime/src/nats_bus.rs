//! NATS-backed `AgentBus` for Phase 8 multi-process deployments.
//!
//! Feature-gated behind `nats`. Single-process dev keeps using `InProcBus`;
//! per-agent systemd-unit deployments load `NatsBus` to share envelopes
//! across processes via the NATS sidecar.
//!
//! Subject layout (per MASTERPLAN §2.2):
//!
//! ```text
//! daimon.<tenant_id>.envelopes
//! ```
//!
//! Tenants are isolated by NKEY scopes at the NATS account level — the
//! Rust client treats subjects as opaque and trusts the server-side ACL.
//! Phase 8.1 adds per-agent + per-topic subjects (`daimon.<tenant>.
//! agent.<id>.<topic>`) when wildcard subscriptions become the limit.
//!
//! Envelopes are encoded as JSON over the NATS wire — same shape as
//! `InProcBus`'s broadcast payload. Switching the encoding to Cap'n Proto
//! / protobuf is a 7.1/8.1 candidate if measurable.

use std::sync::Arc;

use async_nats::{Client, ConnectOptions};
use async_trait::async_trait;
use daimon_core::{AgentEnvelope, BusHandle, CoreError};
use tokio::sync::broadcast;
use tracing::{debug, warn};

use crate::bus::AgentBus;

/// Default mirror-channel capacity. `NatsBus` republishes received NATS
/// messages onto an in-process broadcast so that `subscribe_raw()` returns
/// the same channel shape as `InProcBus` — supervisors don't need to
/// know which bus they're on.
const NATS_MIRROR_CAPACITY: usize = 4096;

#[derive(Clone)]
pub struct NatsBus {
    client: Client,
    subject: String,
    /// Local mirror of envelopes received from NATS, so each agent task
    /// can still call `subscribe_raw()` and get a broadcast receiver.
    mirror: broadcast::Sender<AgentEnvelope>,
}

impl NatsBus {
    /// Connect to a NATS server and start the inbound subscription. The
    /// `tenant_id` becomes part of the subject (one-subject-per-tenant in
    /// Phase 8; finer-grained subjects come in Phase 8.1).
    pub async fn connect(
        url: &str,
        tenant_id: &str,
        nkey_seed: Option<&str>,
    ) -> Result<Self, NatsBusError> {
        let mut opts = ConnectOptions::new()
            .name(format!("daimon-agent-{tenant_id}"))
            .require_tls(false);
        if let Some(seed) = nkey_seed {
            opts = opts.nkey(seed.to_string());
        }
        let client = opts
            .connect(url)
            .await
            .map_err(|e| NatsBusError::Connect(format!("{e}")))?;

        let subject = format!("daimon.{tenant_id}.envelopes");
        let (mirror_tx, _) = broadcast::channel(NATS_MIRROR_CAPACITY);

        // Spawn a forwarder that pulls NATS messages and republishes them
        // onto the in-process broadcast. The lifetime is tied to the
        // NatsBus instance via the Client clone — when the last NatsBus
        // drops, the client drops, the subscription ends.
        let sub_subject = subject.clone();
        let sub_client = client.clone();
        let sub_tx = mirror_tx.clone();
        tokio::spawn(async move {
            let mut sub = match sub_client.subscribe(sub_subject.clone()).await {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, subject = %sub_subject, "nats subscribe failed");
                    return;
                }
            };
            debug!(subject = %sub_subject, "nats subscription active");
            while let Some(msg) = futures_util::StreamExt::next(&mut sub).await {
                match serde_json::from_slice::<AgentEnvelope>(&msg.payload) {
                    Ok(env) => {
                        // broadcast send only fails when there are zero
                        // subscribers — that's fine, the envelope just
                        // gets dropped same as InProcBus does.
                        let _ = sub_tx.send(env);
                    }
                    Err(e) => {
                        warn!(error = %e, "nats envelope decode failed; dropping");
                    }
                }
            }
            debug!(subject = %sub_subject, "nats subscription ended");
        });

        Ok(Self { client, subject, mirror: mirror_tx })
    }

    /// Wrap as a `BusHandle` trait object for handing to agents via context.
    pub fn handle(&self) -> Arc<dyn BusHandle> {
        Arc::new(self.clone())
    }
}

#[async_trait]
impl BusHandle for NatsBus {
    async fn send(&self, env: AgentEnvelope) -> Result<(), CoreError> {
        let bytes = serde_json::to_vec(&env)
            .map_err(|e| CoreError::BusSend(format!("encode envelope: {e}")))?;
        self.client
            .publish(self.subject.clone(), bytes.into())
            .await
            .map_err(|e| CoreError::BusSend(format!("nats publish: {e}")))?;
        Ok(())
    }
}

#[async_trait]
impl AgentBus for NatsBus {
    fn subscribe_raw(&self) -> broadcast::Receiver<AgentEnvelope> {
        self.mirror.subscribe()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NatsBusError {
    #[error("nats connect: {0}")]
    Connect(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use daimon_core::{AgentId, Recipient};
    use semver::VersionReq;
    use serde_json::json;

    /// Ignored — needs a running `nats-server` (start with `just nats-up`).
    #[tokio::test]
    #[ignore]
    async fn roundtrip_envelope_through_nats() {
        let url = std::env::var("DAIMON_NATS_URL")
            .unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
        let bus_a = NatsBus::connect(&url, "test-tenant", None).await.unwrap();
        let bus_b = NatsBus::connect(&url, "test-tenant", None).await.unwrap();
        let mut rx = bus_b.subscribe_raw();
        // Let the mirror subscriber settle.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Use ByCapability shape — serde_json doesn't serialize
        // internally-tagged enums with newtype variants, which is what
        // Recipient::Direct(AgentId) is. ByCapability is a struct
        // variant so it round-trips cleanly. Same constraint applied
        // to PostgresVaultClient payloads in Phase 2c (now
        // serde_json-tagged structs everywhere).
        let env = AgentEnvelope::new(
            AgentId::new("alpha"),
            Recipient::ByCapability {
                name: "noop".into(),
                version_req: VersionReq::parse("*").unwrap(),
            },
            json!({"hello": "nats"}),
        );
        BusHandle::send(&bus_a, env.clone()).await.unwrap();

        let received = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("recv timed out")
            .unwrap();
        assert_eq!(received.correlation_id, env.correlation_id);
        assert_eq!(received.body["hello"], "nats");
    }
}
