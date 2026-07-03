use std::collections::HashMap;
use std::sync::Arc;

use daimon_audit::AuditSink;
use daimon_inventory::{Inventory, InventoryError, TargetMetadata, TargetRef, TransportKind};
use daimon_transport::{OpResult, Transport, TransportError, TransportTarget};
use daimon_vault::{
    CredentialRef, PostgresVaultClient, RefParseError as CredRefParseError, VaultClient, VaultError,
};
use thiserror::Error;
use tracing::{debug, instrument};

use crate::request::ExecRequest;

/// The action broker (D19).
///
/// Agents call `execute(req)` and receive only an `OpResult`. Internally:
/// 1. Look up the target in inventory → get host + port + transport_kind + credential_ref
/// 2. Resolve the credential_ref via vault → typed `Credential`
/// 3. Dispatch via the registered `Transport` for that kind, passing the credential by reference
/// 4. Drop the credential (zeroized via the `Credential` type's `ZeroizeOnDrop`)
/// 5. Return `OpResult` to the agent
///
/// The credential is in scope only for steps 3–4 and never enters worker memory.
///
/// Admin proxy methods (D22/D23/D24 — `vault_*`, `inventory_*`, `audit_*`)
/// live in `daimon-broker/src/admin.rs`. They require the production
/// constructor `with_production_admin` which wires up `vault_admin` (concrete
/// `PostgresVaultClient`) and `audit` (the structured event sink). The
/// legacy `new()` constructor leaves admin disabled (stub usage / tests).
pub struct Broker {
    pub(crate) inventory: Arc<dyn Inventory>,
    pub(crate) vault: Arc<dyn VaultClient>,
    pub(crate) vault_admin: Option<Arc<PostgresVaultClient>>,
    pub(crate) audit: Option<Arc<dyn AuditSink>>,
    pub(crate) transports: HashMap<TransportKind, Arc<dyn Transport>>,
    /// Phase 5 — Guard (KILL switch + policy + approval). When `None`,
    /// every execute proceeds as if read-only (legacy / pre-Guard tests).
    pub(crate) guard: Option<Arc<daimon_guard::Guard>>,
}

impl Broker {
    /// Legacy / test constructor — admin proxy disabled.
    pub fn new(
        inventory: Arc<dyn Inventory>,
        vault: Arc<dyn VaultClient>,
        transports: HashMap<TransportKind, Arc<dyn Transport>>,
    ) -> Self {
        Self {
            inventory,
            vault,
            vault_admin: None,
            audit: None,
            transports,
            guard: None,
        }
    }

    /// Attach a Guard. Builder pattern — typically called once at boot by
    /// the production stack assembler.
    pub fn with_guard(mut self, guard: Arc<daimon_guard::Guard>) -> Self {
        self.guard = Some(guard);
        self
    }

    /// The attached Guard, if any. Exposed so the app can push live tunables
    /// (P6 FR-CFG-06: `guard.approval_timeout_secs`) into the running guard on a
    /// settings write, without threading a separate handle through boot.
    pub fn guard(&self) -> Option<&Arc<daimon_guard::Guard>> {
        self.guard.as_ref()
    }

    /// Production constructor (D22 in-tree vault + D23 audit log + D24 admin).
    ///
    /// `vault` is the concrete `Arc<PostgresVaultClient>` — used both as the
    /// `dyn VaultClient` for the agent `execute` resolve path AND as the
    /// admin-CRUD handle. `audit` is the append-only event sink — every
    /// state-changing admin call emits an event.
    pub fn with_production_admin(
        inventory: Arc<dyn Inventory>,
        vault: Arc<PostgresVaultClient>,
        audit: Arc<dyn AuditSink>,
        transports: HashMap<TransportKind, Arc<dyn Transport>>,
    ) -> Self {
        let vault_dyn: Arc<dyn VaultClient> = vault.clone();
        Self {
            inventory,
            vault: vault_dyn,
            vault_admin: Some(vault),
            audit: Some(audit),
            transports,
            guard: None,
        }
    }

    /// Agent-safe metadata lookup. Returns target info WITHOUT credential ref.
    pub async fn target_metadata(&self, r#ref: &TargetRef) -> Result<TargetMetadata, BrokerError> {
        self.inventory
            .get_metadata(r#ref)
            .await
            .map_err(BrokerError::from)
    }

    /// Agent-safe target listing.
    pub async fn list_targets(
        &self,
        kind_filter: Option<daimon_inventory::TargetKind>,
    ) -> Vec<TargetMetadata> {
        self.inventory.list(kind_filter).await
    }

    /// The protected operation: resolve target, resolve credential, dispatch
    /// transport, zeroize credential, return result. Credentials never leak.
    /// Emits a single audit event (`broker.execute`) capturing the whole
    /// flow — actor, target, credential ref, op summary, result, latency
    /// (D23). On error, the failure stage is recorded in the metadata.
    #[instrument(skip(self, req), fields(target = %req.target_ref, actor = %req.actor_id))]
    pub async fn execute(&self, req: ExecRequest) -> Result<OpResult, BrokerError> {
        let start = std::time::Instant::now();
        let result = self.execute_inner(&req).await;
        self.audit_execute(&req, &result, start).await;
        result
    }

    /// Boot policy-coherence lint over the LIVE capability set (P2 commit 8).
    ///
    /// The P1 predecessor hardcoded the four RouterOS write caps in
    /// `build_production_broker` and ran BEFORE any capability was registered.
    /// This generalizes the check over whatever the supervisor actually spawned:
    /// call it AFTER the registry is populated (in the app boot step) with
    /// `broker.lint_write_capabilities(&harness.capabilities().await)`.
    ///
    /// Two incoherences are fatal (the caller should `exit(1)`):
    ///
    /// 1. **Write auto-allowed** — a WRITE capability (`!cap.is_read()`) whose
    ///    name resolves to `Decision::Allow` under the guard's policy. That would
    ///    let an agent mutate infrastructure with NO approval (the RouterOS
    ///    policy-shadowing class of bug). Writes must be `deny` or
    ///    `require_approval`.
    /// 2. **Dangling compensator** — a capability whose `compensating` action
    ///    names a capability that is NOT present in `caps`. Saga rollback would
    ///    have nothing to dispatch, so the "reversible" claim is a lie.
    ///
    /// The guard/policy stays encapsulated: the broker holds it and this is the
    /// only method that exposes the coherence verdict. With no guard attached
    /// (legacy/test brokers), there is no policy to be incoherent with, so every
    /// write is treated as coherent and only the dangling-compensator structural
    /// check runs.
    pub fn lint_write_capabilities(
        &self,
        caps: &[daimon_core::Capability],
    ) -> Result<(), String> {
        use std::collections::HashSet;

        let present: HashSet<&str> = caps.iter().map(|c| c.name.as_str()).collect();

        for cap in caps {
            // (2) Dangling compensator — applies regardless of read/write and
            // regardless of whether a guard is attached; it's purely structural.
            if let Some(comp) = cap
                .compensating
                .as_ref()
                .filter(|c| !present.contains(c.name.as_str()))
            {
                return Err(format!(
                    "capability `{}` names compensator `{}` which is not present in the \
                     registered capability set — saga rollback would have nothing to dispatch",
                    cap.name, comp.name
                ));
            }

            // (1) Write auto-allowed — only meaningful when a guard/policy is
            // attached. Reads bypass policy, so we only check writes.
            if cap.is_read() {
                continue;
            }
            if let Some(guard) = &self.guard {
                let allowed =
                    guard.policy().evaluate(&cap.name).decision == daimon_guard::Decision::Allow;
                if allowed {
                    return Err(format!(
                        "write capability `{}` resolves to Allow under the shipped policy — \
                         must be deny or require_approval",
                        cap.name
                    ));
                }
            }
        }
        Ok(())
    }

    /// Server-side read-only derivation (H6/H7). The guard's read-only bit is
    /// the sole property of the resolved capability — NEVER a caller/LLM flag.
    /// A request with no `capability_meta` is treated as a WRITE (fail-closed),
    /// so a capability-less call cannot skip policy + approval.
    fn derive_read_only(req: &ExecRequest) -> bool {
        req.capability_meta
            .as_ref()
            .map(|c| c.is_read())
            .unwrap_or(false)
    }

    async fn execute_inner(&self, req: &ExecRequest) -> Result<OpResult, BrokerError> {
        // Step 0: Guard pre-flight (KILL switch + policy + approval).
        if let Some(guard) = &self.guard {
            let cap = req
                .capability
                .clone()
                .unwrap_or_else(|| "broker.execute".to_string());
            let params = serde_json::json!({
                "target_ref": req.target_ref.to_string(),
                "op_summary": op_summary_for(&req.op),
            });
            guard
                .pre_flight(
                    &req.actor_id,
                    &cap,
                    Some(&req.target_ref.to_string()),
                    params,
                    Self::derive_read_only(req),
                )
                .await
                .map_err(BrokerError::from)?;
        }

        // Step 1: inventory lookup (broker-only view).
        let managed = self
            .inventory
            .get_managed(&req.target_ref)
            .await
            .map_err(BrokerError::from)?;

        // Step 2: parse the credential ref and resolve via vault.
        let vref = CredentialRef::parse(&managed.credential_ref)
            .map_err(BrokerError::CredentialRefParse)?;
        let credential = self.vault.resolve(&vref).await.map_err(BrokerError::from)?;

        // Step 3: dispatch transport. `credential` is consumed by reference
        // here; the `Credential` type is `ZeroizeOnDrop`, so the bytes are
        // wiped when this function returns (or panics).
        let transport = self
            .transports
            .get(&managed.transport)
            .ok_or_else(|| BrokerError::NoTransport(managed.transport))?;

        let target = TransportTarget {
            host: managed.host.clone(),
            port: managed.port,
        };

        debug!(
            transport = transport.id(),
            cred_kind = ?credential.kind(),
            "dispatching op"
        );

        // Step 4 + 5: execute, then `credential` drops + zeroizes when this
        // scope exits. `OpResult` does not carry credential material.
        let result = transport
            .execute(&target, &req.op, &credential)
            .await
            .map_err(BrokerError::from)?;

        Ok(result)
    }

    /// Emit one structured audit event per broker.execute call.
    /// Captures op kind, target, and result. Failure stage and message
    /// land in metadata for forensic queries.
    async fn audit_execute(
        &self,
        req: &ExecRequest,
        result: &Result<OpResult, BrokerError>,
        start: std::time::Instant,
    ) {
        let Some(sink) = self.audit.as_ref() else {
            // No audit configured (Broker::new without admin wiring) — silent.
            return;
        };

        // Look up the credential ref the broker resolved against — best-effort,
        // tolerate inventory misses (the error case already records it).
        let credential_ref = self
            .inventory
            .get_managed(&req.target_ref)
            .await
            .ok()
            .map(|m| m.credential_ref);

        let res_tag = match result {
            Ok(_) => daimon_audit::AuditResult::Success,
            Err(_) => daimon_audit::AuditResult::Error,
        };

        let op_summary = op_summary_for(&req.op);

        let mut ev = daimon_audit::NewAuditEvent::new(
            req.actor_id.clone(),
            daimon_audit::ActionKind::BrokerExecute,
            res_tag,
        )
        .with_target(req.target_ref.to_string())
        .with_op_summary(op_summary)
        .with_latency_ms(start.elapsed().as_millis() as u64);

        if let Some(c) = credential_ref {
            ev = ev.with_credential(c);
        }
        if let Err(e) = result {
            ev = ev.with_metadata("error", format!("{e}"));
        }

        if let Err(emit_err) = sink.append(ev).await {
            tracing::warn!(error = %emit_err, "broker.execute audit emit failed");
        }
    }
}

fn op_summary_for(op: &daimon_transport::Op) -> String {
    use daimon_transport::Op;
    match op {
        Op::ShellCommand { command, .. } => {
            let truncated: String = command.chars().take(80).collect();
            format!("ssh:exec:{}", truncated)
        }
        Op::Http { method, path, .. } => format!("http:{method:?}:{path}"),
        Op::SnmpGet { oid } => format!("snmp:get:{oid}"),
        Op::SnmpSet { oid, .. } => format!("snmp:set:{oid}"),
        Op::SnmpWalk { oid_root } => format!("snmp:walk:{oid_root}"),
    }
}

#[derive(Debug, Error)]
pub enum BrokerError {
    #[error("inventory: {0}")]
    Inventory(#[from] InventoryError),
    #[error("vault: {0}")]
    Vault(#[from] VaultError),
    #[error("transport: {0}")]
    Transport(#[from] TransportError),
    #[error("guard: {0}")]
    Guard(#[from] daimon_guard::Error),
    #[error("invalid credential_ref in inventory entry: {0}")]
    CredentialRefParse(CredRefParseError),
    #[error("no transport registered for kind `{0:?}`")]
    NoTransport(TransportKind),
    #[error("admin backend `{0}` not available — broker constructed without production admin wiring")]
    AdminBackendNotAvailable(&'static str),
    #[error("audit: {0}")]
    Audit(String),
}

/// Helper for tests + Phase 2 prototyping — wires a Broker with stub
/// implementations of inventory/vault/transport without needing real
/// Vaultwarden or SSH.
#[cfg(test)]
pub mod test_helpers {
    use super::*;
    use daimon_inventory::InMemoryRegistry;
    use daimon_transport::StubTransport;
    use daimon_vault::StubVaultClient;

    pub fn stub_broker() -> (
        Broker,
        Arc<InMemoryRegistry>,
        Arc<StubVaultClient>,
        Arc<StubTransport>,
    ) {
        let inv = Arc::new(InMemoryRegistry::new());
        let vault = Arc::new(StubVaultClient::new());
        let ssh = Arc::new(StubTransport::new("ssh"));

        let mut transports: HashMap<TransportKind, Arc<dyn Transport>> = HashMap::new();
        transports.insert(TransportKind::Ssh, ssh.clone());

        let broker = Broker::new(inv.clone(), vault.clone(), transports);
        (broker, inv, vault, ssh)
    }
}

#[cfg(test)]
mod read_only_derivation_tests {
    //! AC-P1-10 keystone: the guard's read-only bit is derived SERVER-SIDE
    //! from the capability, never from the caller/LLM-supplied `is_read_only`
    //! flag (H6/H7). A capability-less request is a write (fail-closed).
    use super::*;
    use daimon_inventory::TargetRef;
    use daimon_transport::Op;

    fn cap(json: serde_json::Value) -> daimon_core::Capability {
        serde_json::from_value(json).unwrap()
    }
    fn req() -> ExecRequest {
        ExecRequest::new(
            "actor:test",
            TargetRef::parse("target://x").unwrap(),
            Op::ShellCommand { command: "noop".into(), timeout_secs: 5 },
        )
    }

    #[test]
    fn caller_flag_cannot_make_a_write_read_only() {
        // A write capability, but the caller LIES that it is read-only.
        let write = cap(serde_json::json!({
            "name": "network.routeros.firewall_add_drop_rule",
            "version": "1.0.0",
            "compensating": { "name": "network.routeros.firewall_remove_rule" }
        }));
        let mut r = req().with_capability_meta(write);
        r.is_read_only = true; // the H6/H7 lie
        assert!(
            !Broker::derive_read_only(&r),
            "a write capability must derive read_only=false regardless of the caller flag"
        );
    }

    #[test]
    fn read_capability_derives_read_only() {
        let read = cap(serde_json::json!({
            "name": "network.routeros.system_info", "version": "1.0.0"
        }));
        let r = req().with_capability_meta(read);
        assert!(Broker::derive_read_only(&r));
    }

    #[test]
    fn capability_less_request_is_write_fail_closed() {
        // No capability_meta + caller claims read-only -> still a write.
        let mut r = req();
        r.is_read_only = true;
        assert!(
            !Broker::derive_read_only(&r),
            "a request with no capability descriptor must be treated as a write"
        );
    }
}

#[cfg(test)]
mod lint_write_capabilities_tests {
    //! P2 commit 8 — the boot policy-coherence lint generalized over the LIVE
    //! registry. A synthetic write cap allowed by policy → Err; a dangling
    //! compensator → Err; a coherent set → Ok.
    use super::*;
    use daimon_core::{Capability, CompensatingCapability};
    use daimon_inventory::InMemoryRegistry;
    use daimon_transport::StubTransport;
    use daimon_vault::StubVaultClient;
    use semver::Version;

    /// Build a Broker whose guard uses the given policy TOML. The
    /// `ApprovalQueue` is wired to a lazily-built pool (never contacted — the
    /// lint only reads `guard.policy()`), so no live Postgres is required.
    fn broker_with_policy(policy_toml: &str) -> Broker {
        let inv = Arc::new(InMemoryRegistry::new());
        let vault = Arc::new(StubVaultClient::new());
        let ssh = Arc::new(StubTransport::new("ssh"));
        let mut transports: HashMap<TransportKind, Arc<dyn Transport>> = HashMap::new();
        transports.insert(TransportKind::Ssh, ssh);

        let policy = daimon_guard::PolicyEngine::from_toml_str(policy_toml).expect("policy parses");
        // deadpool builds lazily; the URL is never dialed by the lint.
        let pool = daimon_db::build_pool("postgres://lint@localhost:5432/lint").expect("lazy pool");
        let approvals = daimon_guard::ApprovalQueue::new(pool);
        let guard = Arc::new(daimon_guard::Guard::new(
            daimon_guard::KillState::new(),
            policy,
            approvals,
        ));
        Broker::new(inv, vault, transports).with_guard(guard)
    }

    fn write_cap(name: &str, compensating: Option<&str>) -> Capability {
        Capability {
            name: name.into(),
            version: Version::new(1, 0, 0),
            description: None,
            schema: None,
            compensating: compensating.map(|c| CompensatingCapability {
                name: c.into(),
                version_req: None,
            }),
            irreversible: false,
        }
    }

    #[test]
    fn write_cap_allowed_by_policy_is_rejected() {
        // Policy AUTO-ALLOWS the write → the shadowing bug the lint exists to
        // catch. `.start` is not a read verb, so `is_read()` is false → a write.
        let broker = broker_with_policy(
            r#"
            [[rule]]
            capability = "compute.example.*"
            decision = "allow"
            "#,
        );
        let caps = vec![write_cap("compute.example.vm.start", None)];
        let err = broker.lint_write_capabilities(&caps).unwrap_err();
        assert!(
            err.contains("resolves to Allow"),
            "expected auto-allow rejection, got: {err}"
        );
    }

    #[test]
    fn dangling_compensator_is_rejected() {
        // The write names a compensator that is NOT in the registered set — saga
        // rollback would have nothing to dispatch. Structural: rejected even
        // though the policy require-approvals the write.
        let broker = broker_with_policy(
            r#"
            [[rule]]
            capability = "compute.example.*"
            decision = "require_approval"
            "#,
        );
        let caps = vec![write_cap("compute.example.vm.start", Some("compute.example.vm.stop"))];
        let err = broker.lint_write_capabilities(&caps).unwrap_err();
        assert!(
            err.contains("compute.example.vm.stop") && err.contains("not present"),
            "expected dangling-compensator rejection, got: {err}"
        );
    }

    #[test]
    fn coherent_set_passes() {
        // Every write require-approvals AND every compensator is present.
        let broker = broker_with_policy(
            r#"
            [[rule]]
            capability = "compute.example.*"
            decision = "require_approval"
            "#,
        );
        let caps = vec![
            // read cap — bypasses policy (`_status` is a read verb).
            Capability::read_only("compute.example.vm.status", Version::new(1, 0, 0)),
            // write with its compensator present in the same set.
            write_cap("compute.example.vm.start", Some("compute.example.vm.stop")),
            // the compensator itself (irreversible write).
            Capability {
                name: "compute.example.vm.stop".into(),
                version: Version::new(1, 0, 0),
                description: None,
                schema: None,
                compensating: None,
                irreversible: true,
            },
        ];
        broker
            .lint_write_capabilities(&caps)
            .expect("a coherent set must pass the lint");
    }
}
