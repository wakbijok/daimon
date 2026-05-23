//! OrchestratorService — owns the Postgres pool + broker handle, dispatches
//! step executions, persists state transitions.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use daimon_broker::{Broker, ExecRequest, Op, TargetRef};
use daimon_db::Pool;
use serde_json::{Value as Json, json};
use tracing::{error, info, instrument};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::plan::{Plan, PlanStatus, Step, StepDef, StepStatus};

#[derive(Clone)]
pub struct OrchestratorService {
    pool: Pool,
    broker: Arc<Broker>,
}

impl OrchestratorService {
    pub fn new(pool: Pool, broker: Arc<Broker>) -> Self {
        Self { pool, broker }
    }

    // ---- create + list + get ------------------------------------------------

    #[instrument(skip(self, steps))]
    pub async fn create_plan(
        &self,
        tenant_id: Uuid,
        created_by: Option<Uuid>,
        intent: &str,
        steps: Vec<StepDef>,
    ) -> Result<Plan> {
        let mut client = self.pool.get().await?;
        let txn = client.transaction().await?;

        let plan_row = txn
            .query_one(
                "INSERT INTO public.plans (tenant_id, created_by, intent, status, metadata)
                 VALUES ($1, $2, $3, 'planning', '{}'::jsonb)
                 RETURNING id, tenant_id, created_by, intent, status, metadata,
                           created_at, updated_at, started_at, finished_at",
                &[&tenant_id, &created_by, &intent],
            )
            .await?;
        let plan = row_to_plan(plan_row);

        // First pass — assign step ids in order so depends_on resolution can
        // map indices to ids.
        let step_ids: Vec<Uuid> = (0..steps.len()).map(|_| Uuid::new_v4()).collect();
        for (i, def) in steps.iter().enumerate() {
            let depends_on: Vec<Uuid> = def
                .depends_on_index
                .iter()
                .filter_map(|&idx| step_ids.get(idx).copied())
                .collect();
            txn.execute(
                "INSERT INTO public.plan_steps
                    (id, plan_id, step_index, capability_name, capability_version,
                     target_ref, credential_ref, params, depends_on, status)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'pending')",
                &[
                    &step_ids[i],
                    &plan.id,
                    &(i as i32),
                    &def.capability_name,
                    &def.capability_version,
                    &def.target_ref,
                    &def.credential_ref,
                    &def.params,
                    &depends_on,
                ],
            )
            .await?;
        }
        txn.commit().await?;
        info!(plan_id = %plan.id, steps = steps.len(), "plan persisted");
        Ok(plan)
    }

    pub async fn list_plans(&self, tenant_id: Uuid, limit: i64) -> Result<Vec<Plan>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, tenant_id, created_by, intent, status, metadata,
                        created_at, updated_at, started_at, finished_at
                 FROM public.plans
                 WHERE tenant_id = $1
                 ORDER BY created_at DESC
                 LIMIT $2",
                &[&tenant_id, &limit],
            )
            .await?;
        Ok(rows.into_iter().map(row_to_plan).collect())
    }

    pub async fn get_plan(&self, id: Uuid) -> Result<Option<Plan>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "SELECT id, tenant_id, created_by, intent, status, metadata,
                        created_at, updated_at, started_at, finished_at
                 FROM public.plans WHERE id = $1",
                &[&id],
            )
            .await?;
        Ok(row.map(row_to_plan))
    }

    pub async fn list_steps(&self, plan_id: Uuid) -> Result<Vec<Step>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, plan_id, step_index, capability_name, capability_version,
                        target_ref, credential_ref, params, depends_on, compensating_step_id,
                        status, result, started_at, finished_at
                 FROM public.plan_steps
                 WHERE plan_id = $1
                 ORDER BY step_index ASC",
                &[&plan_id],
            )
            .await?;
        Ok(rows.into_iter().map(row_to_step).collect())
    }

    // ---- execution ----------------------------------------------------------

    /// Execute a previously-created plan. Topological order via depends_on.
    /// Per-step state persisted. Returns the final PlanStatus.
    #[instrument(skip(self))]
    pub async fn run_plan(&self, plan_id: Uuid, actor_id: &str) -> Result<PlanStatus> {
        // Set plan to executing.
        {
            let client = self.pool.get().await?;
            client
                .execute(
                    "UPDATE public.plans
                     SET status = 'executing', started_at = COALESCE(started_at, now()), updated_at = now()
                     WHERE id = $1",
                    &[&plan_id],
                )
                .await?;
        }

        let mut steps = self.list_steps(plan_id).await?;
        if steps.is_empty() {
            self.finish_plan(plan_id, PlanStatus::Succeeded).await?;
            return Ok(PlanStatus::Succeeded);
        }
        let total = steps.len();
        let mut by_id: HashMap<Uuid, usize> = steps
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id, i))
            .collect();
        let _ = total;
        let _ = by_id.len();

        let mut done: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
        let mut failed = false;

        loop {
            // Find a runnable step: pending + all depends_on are in `done`.
            let next_idx = steps.iter().position(|s| {
                matches!(s.status, StepStatus::Pending)
                    && s.depends_on.iter().all(|d| done.contains(d))
            });
            let Some(idx) = next_idx else {
                // No runnable steps. If anything is pending, we have a deadlock
                // (cycle or unmet dependency).
                if steps
                    .iter()
                    .any(|s| matches!(s.status, StepStatus::Pending | StepStatus::Running))
                {
                    failed = true;
                }
                break;
            };
            let step = steps[idx].clone();
            self.set_step_status(step.id, StepStatus::Running, None, true).await?;
            steps[idx].status = StepStatus::Running;

            match self.dispatch_step(&step, actor_id).await {
                Ok(result) => {
                    self.set_step_status(step.id, StepStatus::Succeeded, Some(result.clone()), false)
                        .await?;
                    steps[idx].status = StepStatus::Succeeded;
                    steps[idx].result = Some(result);
                    done.insert(step.id);
                }
                Err(e) => {
                    error!(error = %e, step_id = %step.id, "step failed");
                    let err_json = json!({"error": e.to_string()});
                    self.set_step_status(step.id, StepStatus::Failed, Some(err_json), false)
                        .await?;
                    steps[idx].status = StepStatus::Failed;
                    failed = true;
                    break;
                }
            }
        }

        let final_status = if failed { PlanStatus::Failed } else { PlanStatus::Succeeded };
        self.finish_plan(plan_id, final_status).await?;
        Ok(final_status)
    }

    async fn dispatch_step(&self, step: &Step, actor_id: &str) -> Result<Json> {
        // Phase 6 D1 strictly dispatches Op::ShellCommand for steps with
        // `params.command`. Other Op kinds (Http, Snmp*) ship as the
        // capability tier expands; the dispatch path is identical — just
        // construct a different Op.
        let target_ref_str = step
            .target_ref
            .clone()
            .ok_or_else(|| Error::Dispatch(format!("step {} has no target_ref", step.id)))?;
        let target = TargetRef::parse(&target_ref_str)
            .map_err(|e| Error::Dispatch(format!("parse target_ref: {e}")))?;

        let op = if let Some(cmd) = step.params.get("command").and_then(|v| v.as_str()) {
            Op::ShellCommand {
                command: cmd.to_string(),
                timeout_secs: step
                    .params
                    .get("timeout_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(30) as u32,
            }
        } else {
            return Err(Error::Dispatch(
                "step params must contain a `command` field for Phase 6 D1".into(),
            ));
        };

        let req = ExecRequest::new(
            format!("orchestrator:{}:{}", step.plan_id, step.id),
            target,
            op,
        )
        .with_capability(
            &step.capability_name,
            step.params
                .get("is_read_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        );
        let _ = actor_id; // Phase 6 D1 ties actor to the orchestrator id above.

        let result = self
            .broker
            .execute(req)
            .await
            .map_err(|e| Error::Dispatch(format!("{e}")))?;
        Ok(serde_json::to_value(&result).unwrap_or_else(|_| json!({})))
    }

    async fn set_step_status(
        &self,
        step_id: Uuid,
        status: StepStatus,
        result: Option<Json>,
        set_started: bool,
    ) -> Result<()> {
        let client = self.pool.get().await?;
        let sql = if set_started {
            "UPDATE public.plan_steps
             SET status = $1, started_at = COALESCE(started_at, now())
             WHERE id = $2"
        } else {
            match status {
                StepStatus::Succeeded | StepStatus::Failed | StepStatus::Compensated => {
                    "UPDATE public.plan_steps
                     SET status = $1, result = $3, finished_at = now()
                     WHERE id = $2"
                }
                _ => {
                    "UPDATE public.plan_steps
                     SET status = $1, result = $3
                     WHERE id = $2"
                }
            }
        };
        let result_val = result.unwrap_or(Json::Null);
        if set_started {
            client.execute(sql, &[&status.as_str(), &step_id]).await?;
        } else {
            client.execute(sql, &[&status.as_str(), &step_id, &result_val]).await?;
        }
        Ok(())
    }

    async fn finish_plan(&self, plan_id: Uuid, status: PlanStatus) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE public.plans
                 SET status = $1, finished_at = now(), updated_at = now()
                 WHERE id = $2",
                &[&status.as_str(), &plan_id],
            )
            .await?;
        Ok(())
    }
}

fn row_to_plan(row: tokio_postgres::Row) -> Plan {
    let status_str: String = row.get(4);
    Plan {
        id: row.get(0),
        tenant_id: row.get(1),
        created_by: row.get(2),
        intent: row.get(3),
        status: PlanStatus::from_str(&status_str),
        metadata: row.get(5),
        created_at: row.get(6),
        updated_at: row.get(7),
        started_at: row.get(8),
        finished_at: row.get(9),
    }
}

fn row_to_step(row: tokio_postgres::Row) -> Step {
    let status_str: String = row.get(10);
    Step {
        id: row.get(0),
        plan_id: row.get(1),
        step_index: row.get(2),
        capability_name: row.get(3),
        capability_version: row.get(4),
        target_ref: row.get(5),
        credential_ref: row.get(6),
        params: row.get(7),
        depends_on: row.get(8),
        compensating_step_id: row.get(9),
        status: StepStatus::from_str(&status_str),
        result: row.get(11),
        started_at: row.get(12),
        finished_at: row.get(13),
    }
}
