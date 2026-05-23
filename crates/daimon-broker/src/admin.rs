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
use uuid::Uuid;

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
    #[instrument(skip(self), fields(actor = %actor_id, %id))]
    pub async fn vault_reveal(
        &self,
        actor_id: &str,
        id: Uuid,
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
    ) -> Result<Uuid, BrokerError> {
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
    #[instrument(skip(self, cred), fields(actor = %actor_id, %id))]
    pub async fn vault_update(
        &self,
        actor_id: &str,
        id: Uuid,
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
    #[instrument(skip(self), fields(actor = %actor_id, %id, new_name = %new_name))]
    pub async fn vault_rename(
        &self,
        actor_id: &str,
        id: Uuid,
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
    #[instrument(skip(self), fields(actor = %actor_id, %id))]
    pub async fn vault_delete(&self, actor_id: &str, id: Uuid) -> Result<(), BrokerError> {
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

    /// Get a target's full record — including `credential_ref`. Admin-only
    /// read used by the targets admin UI to pre-fill the edit form. Audited
    /// as `InventoryResolve` since the returned record discloses which vault
    /// entry binds to which target.
    #[instrument(skip(self), fields(actor = %actor_id, ref = %target_ref))]
    pub async fn inventory_get_managed(
        &self,
        actor_id: &str,
        target_ref: &TargetRef,
    ) -> Result<ManagedTarget, BrokerError> {
        let start = Instant::now();
        let result = self.inventory.get_managed(target_ref).await;
        let target_str = target_ref.to_string();
        let cred_str = result.as_ref().ok().map(|t| t.credential_ref.clone());
        self.audit_action(
            actor_id,
            ActionKind::InventoryResolve,
            Some(&target_str),
            cred_str.as_deref(),
            Some("inventory get_managed"),
            &result,
            start,
        )
        .await;
        result.map_err(BrokerError::from)
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

    /// Emit a memory-tier audit event (Phase 3). Used by daimon-app's
    /// `/admin/memory` server-fns to log ingest + retrieve operations.
    /// Action must be one of `ActionKind::MemoryIngest` or `MemoryRetrieve`;
    /// the broker does not constrain this at compile time but admin UI
    /// callers should adhere.
    pub async fn audit_memory_op(
        &self,
        actor_id: &str,
        action: ActionKind,
        target_ref: Option<&str>,
        op_summary: Option<&str>,
        latency_ms: u64,
        success: bool,
        metadata: Vec<(String, String)>,
    ) -> Result<(), BrokerError> {
        let sink = self.require_audit_sink()?;
        let res_tag = if success {
            AuditResult::Success
        } else {
            AuditResult::Error
        };
        let mut ev = NewAuditEvent::new(actor_id.to_string(), action, res_tag)
            .with_latency_ms(latency_ms);
        if let Some(t) = target_ref {
            ev = ev.with_target(t);
        }
        if let Some(s) = op_summary {
            ev = ev.with_op_summary(s);
        }
        for (k, v) in metadata {
            ev = ev.with_metadata(k, v);
        }
        sink.append(ev)
            .await
            .map(|_| ())
            .map_err(|e| BrokerError::Audit(format!("{e}")))
    }

    // -------- Approval inbox proxy (Phase 8) -----------------------------

    /// List pending approval-inbox rows for a tenant, newest first. Errors
    /// if no Guard is attached.
    pub async fn approvals_pending(
        &self,
        tenant_id: uuid::Uuid,
        limit: u32,
    ) -> Result<Vec<daimon_guard::ApprovalRecord>, BrokerError> {
        let guard = self.require_guard()?;
        guard
            .approvals()
            .list_pending(tenant_id, limit)
            .await
            .map_err(|e| BrokerError::Audit(format!("{e}")))
    }

    /// Operator approves or denies a pending row. Returns the updated record.
    pub async fn approvals_decide(
        &self,
        approval_id: uuid::Uuid,
        decided_by: uuid::Uuid,
        status: daimon_guard::ApprovalStatus,
    ) -> Result<daimon_guard::ApprovalRecord, BrokerError> {
        let guard = self.require_guard()?;
        guard
            .approvals()
            .decide(approval_id, decided_by, status)
            .await
            .map_err(|e| BrokerError::Audit(format!("{e}")))
    }

    // -------- Internal helpers --------------------------------------------

    fn require_vault_admin(&self) -> Result<&Arc<daimon_vault::PostgresVaultClient>, BrokerError> {
        self.vault_admin
            .as_ref()
            .ok_or(BrokerError::AdminBackendNotAvailable("vault"))
    }

    fn require_audit_sink(&self) -> Result<&Arc<dyn daimon_audit::AuditSink>, BrokerError> {
        self.audit.as_ref().ok_or(BrokerError::AdminBackendNotAvailable("audit"))
    }

    fn require_guard(&self) -> Result<&Arc<daimon_guard::Guard>, BrokerError> {
        self.guard.as_ref().ok_or(BrokerError::AdminBackendNotAvailable("guard"))
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


// Phase 2c D3b: SQLite-backed admin test fixtures (SqliteRegistry,
// SqliteVaultClient, SqliteAuditSink) were removed in the rip-and-replace.
// D19 credential opacity, audit emission on failure, inventory_get_managed
// leak check etc. are exercised by tests/multi_tenant_isolation.rs (D8)
// against a live Postgres test schema.

