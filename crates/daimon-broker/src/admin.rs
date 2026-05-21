//! Admin proxy surface on the broker (D22/D23/D24).
//!
//! `daimon-app` calls these methods *exclusively* — per D21, `daimon-app`
//! does NOT depend on `daimon-vault`, `daimon-inventory`, `daimon-transport`,
//! or `daimon-audit` directly. The broker is the single integration point.
//!
//! Every state-changing admin call emits a structured audit event (D23) so
//! every credential reveal, every target upsert, every inventory delete is
//! attributable to the calling admin user.

use std::sync::Arc;
use std::time::Instant;

use daimon_audit::{ActionKind, AuditEvent, AuditFilter, AuditResult, NewAuditEvent};
use daimon_inventory::{ManagedTarget, TargetMetadata, TargetRef};
use daimon_vault::{Credential, CredentialMetadata};
use tracing::{debug, instrument, warn};

use crate::broker::{Broker, BrokerError};

impl Broker {
    // -------- Vault admin proxy ---------------------------------------------

    /// List credential metadata (no secret material). Audit-logged as a
    /// privileged read at INFO; no `AuditResult::Denied` path because the
    /// caller is already gated by `require_admin()` at the daimon-app layer.
    #[instrument(skip(self), fields(actor = %actor_id))]
    pub async fn vault_list_metadata(
        &self,
        actor_id: &str,
    ) -> Result<Vec<CredentialMetadata>, BrokerError> {
        let admin = self.require_vault_admin()?;
        let start = Instant::now();
        let result = admin.list_metadata().await;
        self.audit_action(
            actor_id,
            ActionKind::VaultResolve,
            None,
            None,
            Some("vault list"),
            &result,
            start,
        )
        .await;
        result.map_err(BrokerError::from)
    }

    /// Reveal a credential's full payload by id. Always audited — this is the
    /// most sensitive admin operation.
    #[instrument(skip(self), fields(actor = %actor_id, id))]
    pub async fn vault_reveal(
        &self,
        actor_id: &str,
        id: i64,
    ) -> Result<Credential, BrokerError> {
        let admin = self.require_vault_admin()?;
        let start = Instant::now();
        let result = admin.reveal(id).await;
        self.audit_action(
            actor_id,
            ActionKind::VaultReveal,
            None,
            Some(&format!("id={id}")),
            Some(&format!("revealed credential id={id}")),
            &result,
            start,
        )
        .await;
        result.map_err(BrokerError::from)
    }

    /// Create a credential.
    #[instrument(skip(self, cred), fields(actor = %actor_id, name = %name))]
    pub async fn vault_create(
        &self,
        actor_id: &str,
        name: &str,
        cred: Credential,
    ) -> Result<i64, BrokerError> {
        let admin = self.require_vault_admin()?;
        let kind = cred.kind();
        let start = Instant::now();
        let result = admin.create(name, cred).await;
        self.audit_action(
            actor_id,
            ActionKind::VaultCreate,
            None,
            Some(&format!("vault://{name}")),
            Some(&format!("created credential kind={:?}", kind)),
            &result,
            start,
        )
        .await;
        result.map_err(BrokerError::from)
    }

    /// Update an existing credential's payload by id.
    #[instrument(skip(self, cred), fields(actor = %actor_id, id))]
    pub async fn vault_update(
        &self,
        actor_id: &str,
        id: i64,
        cred: Credential,
    ) -> Result<(), BrokerError> {
        let admin = self.require_vault_admin()?;
        let start = Instant::now();
        let result = admin.update(id, cred).await;
        self.audit_action(
            actor_id,
            ActionKind::VaultUpdate,
            None,
            Some(&format!("id={id}")),
            Some(&format!("updated credential id={id}")),
            &result,
            start,
        )
        .await;
        result.map_err(BrokerError::from)
    }

    /// Rename a credential.
    #[instrument(skip(self), fields(actor = %actor_id, id, new_name = %new_name))]
    pub async fn vault_rename(
        &self,
        actor_id: &str,
        id: i64,
        new_name: &str,
    ) -> Result<(), BrokerError> {
        let admin = self.require_vault_admin()?;
        let start = Instant::now();
        let result = admin.rename(id, new_name).await;
        self.audit_action(
            actor_id,
            ActionKind::VaultRename,
            None,
            Some(&format!("vault://{new_name}")),
            Some(&format!("renamed credential id={id} to {new_name}")),
            &result,
            start,
        )
        .await;
        result.map_err(BrokerError::from)
    }

    /// Delete a credential.
    #[instrument(skip(self), fields(actor = %actor_id, id))]
    pub async fn vault_delete(&self, actor_id: &str, id: i64) -> Result<(), BrokerError> {
        let admin = self.require_vault_admin()?;
        let start = Instant::now();
        let result = admin.delete(id).await;
        self.audit_action(
            actor_id,
            ActionKind::VaultDelete,
            None,
            Some(&format!("id={id}")),
            Some(&format!("deleted credential id={id}")),
            &result,
            start,
        )
        .await;
        result.map_err(BrokerError::from)
    }

    // -------- Inventory admin proxy ----------------------------------------

    /// List managed targets (metadata only — never carries credential_ref).
    #[instrument(skip(self), fields(actor = %actor_id))]
    pub async fn inventory_list(
        &self,
        actor_id: &str,
        kind_filter: Option<daimon_inventory::TargetKind>,
    ) -> Vec<TargetMetadata> {
        let _ = actor_id; // No audit for inventory list (already non-secret)
        self.inventory.list(kind_filter).await
    }

    /// Get a target's metadata by ref.
    pub async fn inventory_get_metadata(
        &self,
        _actor_id: &str,
        target_ref: &TargetRef,
    ) -> Result<TargetMetadata, BrokerError> {
        self.inventory
            .get_metadata(target_ref)
            .await
            .map_err(BrokerError::from)
    }

    /// Upsert a managed target (create or update).
    #[instrument(skip(self, target), fields(actor = %actor_id, ref = %target.r#ref))]
    pub async fn inventory_upsert(
        &self,
        actor_id: &str,
        target: ManagedTarget,
    ) -> Result<(), BrokerError> {
        let target_ref = target.r#ref.to_string();
        let start = Instant::now();
        let result = self.inventory.upsert(target).await;
        self.audit_action(
            actor_id,
            ActionKind::InventoryUpsert,
            Some(&target_ref),
            None,
            Some("inventory upsert"),
            &result,
            start,
        )
        .await;
        result.map_err(BrokerError::from)
    }

    /// Remove a managed target.
    #[instrument(skip(self), fields(actor = %actor_id, ref = %target_ref))]
    pub async fn inventory_remove(
        &self,
        actor_id: &str,
        target_ref: &TargetRef,
    ) -> Result<(), BrokerError> {
        let start = Instant::now();
        let result = self.inventory.remove(target_ref).await;
        self.audit_action(
            actor_id,
            ActionKind::InventoryRemove,
            Some(&target_ref.to_string()),
            None,
            Some("inventory remove"),
            &result,
            start,
        )
        .await;
        result.map_err(BrokerError::from)
    }

    // -------- Audit query proxy -------------------------------------------

    /// Paged audit query (admin UI).
    pub async fn audit_query(
        &self,
        _actor_id: &str,
        filter: &AuditFilter,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEvent>, BrokerError> {
        let sink = self.require_audit_sink()?;
        sink.query(filter, limit, offset)
            .await
            .map_err(|e| BrokerError::Audit(format!("{e}")))
    }

    /// Total count matching a filter (paged-UI counter).
    pub async fn audit_count(
        &self,
        _actor_id: &str,
        filter: &AuditFilter,
    ) -> Result<u64, BrokerError> {
        let sink = self.require_audit_sink()?;
        sink.count(filter)
            .await
            .map_err(|e| BrokerError::Audit(format!("{e}")))
    }

    // -------- Internal helpers --------------------------------------------

    fn require_vault_admin(&self) -> Result<&Arc<daimon_vault::SqliteVaultClient>, BrokerError> {
        self.vault_admin
            .as_ref()
            .ok_or(BrokerError::AdminBackendNotAvailable("vault"))
    }

    fn require_audit_sink(&self) -> Result<&Arc<dyn daimon_audit::AuditSink>, BrokerError> {
        self.audit.as_ref().ok_or(BrokerError::AdminBackendNotAvailable("audit"))
    }

    async fn audit_action<T, E: std::fmt::Display>(
        &self,
        actor_id: &str,
        action: ActionKind,
        target_ref: Option<&str>,
        credential_ref: Option<&str>,
        op_summary: Option<&str>,
        result: &Result<T, E>,
        start: Instant,
    ) {
        let Some(sink) = self.audit.as_ref() else {
            debug!("no audit sink configured — skipping audit emission");
            return;
        };
        let res_tag = match result {
            Ok(_) => AuditResult::Success,
            Err(_) => AuditResult::Error,
        };
        let mut ev = NewAuditEvent::new(actor_id.to_string(), action, res_tag)
            .with_latency_ms(start.elapsed().as_millis() as u64);
        if let Some(t) = target_ref {
            ev = ev.with_target(t);
        }
        if let Some(c) = credential_ref {
            ev = ev.with_credential(c);
        }
        if let Some(s) = op_summary {
            ev = ev.with_op_summary(s);
        }
        if let Err(e) = result {
            ev = ev.with_metadata("error", format!("{e}"));
        }
        if let Err(emit_err) = sink.append(ev).await {
            warn!(error = %emit_err, "audit emit failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::Broker;
    use daimon_audit::SqliteAuditSink;
    use daimon_inventory::{
        InMemoryRegistry, ManagedTarget, SqliteRegistry, TargetKind, TargetRef, TransportKind,
    };
    use daimon_transport::StubTransport;
    use daimon_vault::{Credential, MasterKey, SqliteVaultClient};
    use std::collections::{BTreeMap, HashMap};
    use std::sync::Arc;

    async fn admin_broker() -> Broker {
        let inv = Arc::new(SqliteRegistry::in_memory().unwrap());
        let vault = Arc::new(SqliteVaultClient::in_memory(MasterKey::from_bytes([7u8; 32])).unwrap());
        let audit: Arc<dyn daimon_audit::AuditSink> = Arc::new(SqliteAuditSink::in_memory().unwrap());
        let ssh: Arc<dyn daimon_transport::Transport> = Arc::new(StubTransport::new("ssh"));
        let mut transports: HashMap<TransportKind, Arc<dyn daimon_transport::Transport>> =
            HashMap::new();
        transports.insert(TransportKind::Ssh, ssh);
        Broker::with_production_admin(inv, vault, audit, transports)
    }

    #[tokio::test]
    async fn vault_create_then_list_then_reveal_round_trip() {
        let b = admin_broker().await;
        let id = b
            .vault_create(
                "user:arif",
                "mikrotik-edge",
                Credential::ApiToken {
                    token: "secret-token".into(),
                },
            )
            .await
            .unwrap();
        let list = b.vault_list_metadata("user:arif").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].name, "mikrotik-edge");
        let cred = b.vault_reveal("user:arif", id).await.unwrap();
        match cred {
            Credential::ApiToken { ref token } => assert_eq!(token, "secret-token"),
            ref other => panic!("expected ApiToken, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn vault_admin_calls_without_backend_return_unavailable() {
        // Build a broker WITHOUT admin backends — like the original Broker::new.
        let inv = Arc::new(InMemoryRegistry::new());
        let vault: Arc<dyn daimon_vault::VaultClient> = Arc::new(daimon_vault::StubVaultClient::new());
        let ssh: Arc<dyn daimon_transport::Transport> = Arc::new(StubTransport::new("ssh"));
        let mut transports: HashMap<TransportKind, Arc<dyn daimon_transport::Transport>> =
            HashMap::new();
        transports.insert(TransportKind::Ssh, ssh);
        let b = Broker::new(inv, vault, transports);

        let err = b.vault_list_metadata("user:x").await.unwrap_err();
        match err {
            BrokerError::AdminBackendNotAvailable(name) => assert_eq!(name, "vault"),
            other => panic!("expected AdminBackendNotAvailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn inventory_upsert_then_list_round_trip() {
        let b = admin_broker().await;
        let t = ManagedTarget {
            r#ref: TargetRef::parse("target://mikrotik-edge").unwrap(),
            kind: TargetKind::Network,
            transport: TransportKind::Ssh,
            host: "10.100.10.1".into(),
            port: 22,
            credential_ref: "vault://mikrotik-edge".into(),
            labels: BTreeMap::new(),
            capabilities: vec![],
        };
        b.inventory_upsert("user:arif", t.clone()).await.unwrap();
        let list = b.inventory_list("user:arif", None).await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].host, "10.100.10.1");
        // Credential ref must not leak through the metadata view.
        let json = serde_json::to_string(&list).unwrap();
        assert!(!json.contains("vault://"));
    }

    #[tokio::test]
    async fn audit_events_recorded_for_vault_actions() {
        let b = admin_broker().await;
        let id = b
            .vault_create(
                "user:arif",
                "audit-test",
                Credential::ApiToken {
                    token: "x".into(),
                },
            )
            .await
            .unwrap();
        let _ = b.vault_reveal("user:arif", id).await.unwrap();
        let _ = b.vault_update(
            "user:arif",
            id,
            Credential::ApiToken { token: "y".into() },
        )
        .await
        .unwrap();
        let _ = b.vault_delete("user:arif", id).await.unwrap();

        let events = b
            .audit_query("user:arif", &AuditFilter::default(), 50, 0)
            .await
            .unwrap();
        let actions: Vec<_> = events.iter().map(|e| e.action).collect();
        assert!(actions.contains(&ActionKind::VaultDelete));
        assert!(actions.contains(&ActionKind::VaultUpdate));
        assert!(actions.contains(&ActionKind::VaultReveal));
        assert!(actions.contains(&ActionKind::VaultCreate));
        // All events attributed to the actor.
        assert!(events.iter().all(|e| e.actor_id == "user:arif"));
    }

    #[tokio::test]
    async fn audit_events_recorded_for_inventory_actions() {
        let b = admin_broker().await;
        let t = ManagedTarget {
            r#ref: TargetRef::parse("target://test").unwrap(),
            kind: TargetKind::Host,
            transport: TransportKind::Ssh,
            host: "x".into(),
            port: 22,
            credential_ref: "vault://x".into(),
            labels: BTreeMap::new(),
            capabilities: vec![],
        };
        b.inventory_upsert("user:arif", t.clone()).await.unwrap();
        b.inventory_remove("user:arif", &t.r#ref).await.unwrap();
        let filter = AuditFilter {
            target_ref: Some("target://test".into()),
            ..Default::default()
        };
        let events = b.audit_query("user:arif", &filter, 50, 0).await.unwrap();
        assert_eq!(events.len(), 2);
        let actions: Vec<_> = events.iter().map(|e| e.action).collect();
        assert!(actions.contains(&ActionKind::InventoryUpsert));
        assert!(actions.contains(&ActionKind::InventoryRemove));
    }

    #[tokio::test]
    async fn vault_delete_audit_records_error_on_missing_id() {
        let b = admin_broker().await;
        let _ = b.vault_delete("user:arif", 99999).await; // ignore err
        let events = b
            .audit_query("user:arif", &AuditFilter::default(), 10, 0)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, ActionKind::VaultDelete);
        assert_eq!(events[0].result, AuditResult::Error);
        assert!(
            events[0]
                .metadata
                .get("error")
                .map(|s| s.contains("NotFound") || s.contains("id=99999"))
                .unwrap_or(false),
            "expected error metadata, got {:?}",
            events[0].metadata
        );
    }
}
