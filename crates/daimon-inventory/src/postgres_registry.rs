//! Postgres-backed `Inventory` impl (Phase 2c D3b).
//!
//! Replaces `SqliteRegistry` for production. Tenant-scoped via the Pool —
//! each constructed `PostgresRegistry` instance is bound to a single
//! tenant_id. Multi-tenant routing lands in D6.

use async_trait::async_trait;
use daimon_db::Pool;
use std::collections::BTreeMap;
use tracing::instrument;
use uuid::Uuid;

use crate::refspec::TargetRef;
use crate::registry::{Inventory, InventoryError};
use crate::target::{ManagedTarget, TargetKind, TargetMetadata, TransportKind};

#[derive(Clone)]
pub struct PostgresRegistry {
    pool: Pool,
    tenant_id: Uuid,
}

impl PostgresRegistry {
    pub fn new(pool: Pool, tenant_id: Uuid) -> Self {
        Self { pool, tenant_id }
    }

    pub async fn count(&self) -> Result<u64, InventoryError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| InventoryError::Other(format!("pool: {e}")))?;
        let row = client
            .query_one(
                "SELECT COUNT(*) FROM inventory.targets WHERE tenant_id = $1",
                &[&self.tenant_id],
            )
            .await
            .map_err(|e| InventoryError::Other(format!("count: {e}")))?;
        let n: i64 = row.get(0);
        Ok(n as u64)
    }
}

#[async_trait]
impl Inventory for PostgresRegistry {
    #[instrument(skip(self), fields(ref = %r#ref))]
    async fn get_metadata(&self, r#ref: &TargetRef) -> Result<TargetMetadata, InventoryError> {
        let mt = self.get_managed(r#ref).await?;
        Ok(mt.metadata())
    }

    #[instrument(skip(self), fields(ref = %r#ref))]
    async fn get_managed(&self, r#ref: &TargetRef) -> Result<ManagedTarget, InventoryError> {
        let key = r#ref.to_string();
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| InventoryError::Other(format!("pool: {e}")))?;
        let row = client
            .query_opt(
                "SELECT target_ref, kind, transport, host, port, credential_ref, labels, capabilities
                 FROM inventory.targets
                 WHERE tenant_id = $1 AND target_ref = $2",
                &[&self.tenant_id, &key],
            )
            .await
            .map_err(|e| InventoryError::Other(format!("get: {e}")))?
            .ok_or_else(|| InventoryError::NotFound(key.clone()))?;
        row_to_managed(&row)
    }

    async fn list(&self, kind_filter: Option<TargetKind>) -> Vec<TargetMetadata> {
        let client = match self.pool.get().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "inventory list pool failed");
                return Vec::new();
            }
        };
        let rows = match kind_filter {
            Some(k) => {
                let kind_str = kind_to_str(k);
                client
                    .query(
                        "SELECT target_ref, kind, transport, host, port, credential_ref, labels, capabilities
                         FROM inventory.targets
                         WHERE tenant_id = $1 AND kind = $2
                         ORDER BY target_ref",
                        &[&self.tenant_id, &kind_str],
                    )
                    .await
            }
            None => {
                client
                    .query(
                        "SELECT target_ref, kind, transport, host, port, credential_ref, labels, capabilities
                         FROM inventory.targets
                         WHERE tenant_id = $1
                         ORDER BY target_ref",
                        &[&self.tenant_id],
                    )
                    .await
            }
        };
        match rows {
            Ok(rows) => rows
                .iter()
                .filter_map(|r| row_to_managed(r).ok().map(|m| m.metadata()))
                .collect(),
            Err(e) => {
                tracing::error!(error = %e, "inventory list query failed");
                Vec::new()
            }
        }
    }

    #[instrument(skip(self, target), fields(ref = %target.r#ref))]
    async fn upsert(&self, target: ManagedTarget) -> Result<(), InventoryError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| InventoryError::Other(format!("pool: {e}")))?;
        let labels: serde_json::Value =
            serde_json::to_value(&target.labels).map_err(|e| InventoryError::Other(format!("labels: {e}")))?;
        let caps: serde_json::Value =
            serde_json::to_value(&target.capabilities).map_err(|e| InventoryError::Other(format!("caps: {e}")))?;
        let port_i32 = target.port as i32;
        let kind_str = kind_to_str(target.kind);
        let transport_str = transport_to_str(target.transport);
        let ref_str = target.r#ref.to_string();
        client
            .execute(
                "INSERT INTO inventory.targets
                    (tenant_id, target_ref, kind, transport, host, port,
                     credential_ref, labels, capabilities)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT (tenant_id, target_ref) DO UPDATE
                    SET kind = EXCLUDED.kind,
                        transport = EXCLUDED.transport,
                        host = EXCLUDED.host,
                        port = EXCLUDED.port,
                        credential_ref = EXCLUDED.credential_ref,
                        labels = EXCLUDED.labels,
                        capabilities = EXCLUDED.capabilities,
                        updated_at = now()",
                &[
                    &self.tenant_id,
                    &ref_str,
                    &kind_str,
                    &transport_str,
                    &target.host,
                    &port_i32,
                    &target.credential_ref,
                    &labels,
                    &caps,
                ],
            )
            .await
            .map_err(|e| InventoryError::Other(format!("upsert: {e}")))?;
        Ok(())
    }

    #[instrument(skip(self), fields(ref = %r#ref))]
    async fn remove(&self, r#ref: &TargetRef) -> Result<(), InventoryError> {
        let key = r#ref.to_string();
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| InventoryError::Other(format!("pool: {e}")))?;
        let n = client
            .execute(
                "DELETE FROM inventory.targets WHERE tenant_id = $1 AND target_ref = $2",
                &[&self.tenant_id, &key],
            )
            .await
            .map_err(|e| InventoryError::Other(format!("remove: {e}")))?;
        if n == 0 {
            return Err(InventoryError::NotFound(key));
        }
        Ok(())
    }
}

fn row_to_managed(row: &tokio_postgres::Row) -> Result<ManagedTarget, InventoryError> {
    let target_ref_str: String = row.get(0);
    let kind_str: String = row.get(1);
    let transport_str: String = row.get(2);
    let host: String = row.get(3);
    let port_i32: i32 = row.get(4);
    let credential_ref: String = row.get(5);
    let labels_val: serde_json::Value = row.get(6);
    let caps_val: serde_json::Value = row.get(7);

    let r#ref = TargetRef::parse(&target_ref_str)
        .map_err(|e| InventoryError::Other(format!("parse ref: {e}")))?;
    let kind = parse_kind(&kind_str)?;
    let transport = parse_transport(&transport_str)?;
    let labels: BTreeMap<String, String> = serde_json::from_value(labels_val).unwrap_or_default();
    let capabilities: Vec<String> = serde_json::from_value(caps_val).unwrap_or_default();
    if !(0..=u16::MAX as i32).contains(&port_i32) {
        return Err(InventoryError::Other(format!("port out of range: {port_i32}")));
    }
    Ok(ManagedTarget {
        r#ref,
        kind,
        transport,
        host,
        port: port_i32 as u16,
        credential_ref,
        labels,
        capabilities,
    })
}

fn kind_to_str(k: TargetKind) -> &'static str {
    match k {
        TargetKind::Platform => "platform",
        TargetKind::Network => "network",
        TargetKind::Host => "host",
        TargetKind::App => "app",
    }
}

fn parse_kind(s: &str) -> Result<TargetKind, InventoryError> {
    match s {
        "platform" => Ok(TargetKind::Platform),
        "network" => Ok(TargetKind::Network),
        "host" => Ok(TargetKind::Host),
        "app" => Ok(TargetKind::App),
        other => Err(InventoryError::Other(format!("unknown kind: {other}"))),
    }
}

fn transport_to_str(t: TransportKind) -> &'static str {
    match t {
        TransportKind::Ssh => "ssh",
        TransportKind::Rest => "rest",
        TransportKind::Snmp => "snmp",
        TransportKind::Grpc => "grpc",
    }
}

fn parse_transport(s: &str) -> Result<TransportKind, InventoryError> {
    match s {
        "ssh" => Ok(TransportKind::Ssh),
        "rest" => Ok(TransportKind::Rest),
        "snmp" => Ok(TransportKind::Snmp),
        "grpc" => Ok(TransportKind::Grpc),
        other => Err(InventoryError::Other(format!("unknown transport: {other}"))),
    }
}
