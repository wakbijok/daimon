//! Tenant-session helpers for the RLS-enforced relational tier.
//!
//! The V011 policies key off two GUCs:
//! - `app.tenant_id` — UUID of the tenant the caller is acting on behalf of
//! - `app.role` — `cluster_admin` to bypass tenant scoping; any other value
//!   (or unset) defaults to tenant-only access
//!
//! Production deployments run daimon-app under a non-owner role so RLS is
//! enforced; dev runs as the owner role and RLS becomes passthrough.
//! Either way, callers should still set these GUCs so the production
//! posture is exercised in dev too.

use deadpool_postgres::Transaction;
use tokio_postgres::error::Error as PgError;
use uuid::Uuid;

use crate::Pool;

/// Set the per-transaction RLS GUCs. Caller must hold a transaction so
/// `SET LOCAL` semantics apply; the values clear at COMMIT/ROLLBACK.
pub async fn set_tenant_context(
    txn: &Transaction<'_>,
    tenant_id: Uuid,
    role: &str,
) -> Result<(), PgError> {
    txn.execute(
        "SELECT set_config('app.tenant_id', $1, true)",
        &[&tenant_id.to_string()],
    )
    .await?;
    txn.execute("SELECT set_config('app.role', $1, true)", &[&role])
        .await?;
    Ok(())
}

/// Acquire a client, open a transaction with the tenant context set, run
/// the closure, and commit. Closure runs inside the transaction so the
/// SET LOCAL GUCs apply for every query made through `txn`.
pub async fn with_tenant<F, Fut, T>(
    pool: &Pool,
    tenant_id: Uuid,
    role: &str,
    f: F,
) -> anyhow::Result<T>
where
    F: for<'a> FnOnce(&'a Transaction<'a>) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let mut client = pool.get().await.map_err(|e| anyhow::anyhow!("pool: {e}"))?;
    let txn = client.transaction().await?;
    set_tenant_context(&txn, tenant_id, role).await?;
    let out = f(&txn).await?;
    txn.commit().await?;
    Ok(out)
}
