//! OrchestratorService — owns the Postgres pool + broker handle, dispatches
//! step executions, persists state transitions.

use std::sync::Arc;
use std::time::Duration;

use daimon_core::AgentId;
use daimon_db::Pool;
use daimon_graph::{
    GraphClient, GraphPlan, GraphPlanStep, TargetRef as GraphTargetRef,
};
use daimon_llm::{ChatMessage, CompletionRequest, LlmClient};
use daimon_memory::{MemoryService, TypedBody, TypedRecord};
use daimon_runtime::Dispatcher;
use semver::VersionReq;
use serde_json::{Value as Json, json};
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::plan::{Plan, PlanStatus, Step, StepDef, StepStatus};

/// Per-step bus dispatch budget. Longer than chat's 35s: a plan step may block
/// inside the worker's `broker.execute → Guard → ApprovalQueue::wait_for_decision`
/// while an operator decides.
const STEP_DISPATCH_TIMEOUT: Duration = Duration::from_secs(60);

const PLANNER_SYSTEM_PROMPT: &str = "\
You are dAImon's plan-emitter. Given an operator intent and the capability \
catalog, return a JSON object with shape:
{
  \"steps\": [
    {
      \"capability_name\": \"network.routeros.firewall_add_drop_rule\",
      \"capability_version\": \"1.0.0\",
      \"target_ref\": \"target://mikrotik-edge\",
      \"params\": {\"dst_address\": \"185.60.216.0/24\", \"comment\": \"block\"},
      \"depends_on_index\": []
    }
  ]
}

Return JSON only, no prose. Use capability_name from the catalog EXACTLY. \
Put the capability's TYPED parameters (as named in its input schema) under \
`params` — the driver renders the device command from them; do NOT emit a raw \
CLI string or a `params.command` field. Read capabilities take only \
`target_ref` and no params. (Do NOT emit is_read_only — the platform derives \
read/write disposition from the capability itself.) Step `depends_on_index` is \
a list of 0-based indices of earlier steps that must complete before this one \
runs. Use the minimum number of steps that satisfies the intent.";

#[derive(Clone)]
pub struct OrchestratorService {
    pool: Pool,
    /// P2 commit 6: dispatch runs over the bus, not the broker directly. The
    /// dispatcher is built from the SAME bus+registry as the app's Harness
    /// (passed in from daimon-app main.rs), so a plan-initiated write resolves
    /// the same driver, hits the same Guard gate, and lands the same audit
    /// event as a chat-initiated write. The orchestrator keeps only the pool
    /// for plan persistence — it never touches vault/transport (D21).
    dispatcher: Dispatcher,
    /// Phase 8 graph tier (NornicDB). Postgres is the canonical store for
    /// plans; the graph holds the same plan + capability + target edges
    /// for cross-reference queries (blast-radius, lineage). Mirror writes
    /// are best-effort — a graph-side failure logs a warning but doesn't
    /// fail the plan creation.
    graph: Option<Arc<dyn GraphClient>>,
    /// P3 commit 10 — long-term memory. On a TERMINAL plan state,
    /// `finish_plan` captures a typed record (Succeeded → Decision,
    /// Failed → Incident) AFTER the status UPDATE + audit. Optional: `None`
    /// (the default) skips capture entirely. Every capture is fail-soft — a
    /// memory fault logs and is swallowed, never failing the plan transition
    /// (audit is truth, memory is the aid).
    memory: Option<Arc<dyn MemoryService>>,
}

impl OrchestratorService {
    pub fn new(pool: Pool, dispatcher: Dispatcher) -> Self {
        Self { pool, dispatcher, graph: None, memory: None }
    }

    /// Attach a graph client for plan-DAG mirroring. See struct comment.
    pub fn with_graph(mut self, graph: Arc<dyn GraphClient>) -> Self {
        self.graph = Some(graph);
        self
    }

    /// Attach a memory service for terminal-state typed-record capture. See
    /// the `memory` field comment. Mirrors `with_graph`.
    pub fn with_memory(mut self, memory: Arc<dyn MemoryService>) -> Self {
        self.memory = Some(memory);
        self
    }

    // ---- create + list + get ------------------------------------------------

    #[instrument(skip(self, steps))]
    pub async fn create_plan(
        &self,
        created_by: Option<Uuid>,
        intent: &str,
        steps: Vec<StepDef>,
    ) -> Result<Plan> {
        let mut client = self.pool.get().await?;
        let txn = client.transaction().await?;

        let plan_row = txn
            .query_one(
                "INSERT INTO public.plans (created_by, intent, status, metadata)
                 VALUES ($1, $2, 'planning', '{}'::jsonb)
                 RETURNING id, created_by, intent, status, metadata,
                           created_at, updated_at, started_at, finished_at",
                &[&created_by, &intent],
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

        // Phase 8: best-effort mirror to NornicDB. Skip steps that have no
        // target_ref (they can't form an edge in the graph schema).
        if let Some(graph) = &self.graph {
            let graph_steps: Vec<GraphPlanStep> = steps
                .iter()
                .enumerate()
                .filter_map(|(i, def)| {
                    def.target_ref.as_ref().map(|tref| {
                        let depends_on: Vec<Uuid> = def
                            .depends_on_index
                            .iter()
                            .filter_map(|&idx| step_ids.get(idx).copied())
                            .collect();
                        GraphPlanStep {
                            id: step_ids[i],
                            capability_name: def.capability_name.clone(),
                            capability_version: def.capability_version.clone(),
                            target_ref: GraphTargetRef::from(tref.as_str()),
                            depends_on,
                        }
                    })
                })
                .collect();
            let graph_plan = GraphPlan {
                id: plan.id,
                intent: plan.intent.clone(),
                created_at: plan.created_at,
                steps: graph_steps,
            };
            if let Err(e) = graph.persist_plan(&graph_plan).await {
                warn!(plan_id = %plan.id, error = %e, "graph mirror failed (non-fatal)");
            } else {
                info!(plan_id = %plan.id, "graph mirror persisted");
            }
        }

        Ok(plan)
    }

    /// Phase 6 D2 — LLM-emitted DAG. Builds a planner prompt with the
    /// supplied capability catalog + intent, asks the LLM for a JSON plan,
    /// parses + persists via create_plan. Caller is responsible for picking
    /// the LLM client (typically Anthropic from $ANTHROPIC_API_KEY).
    #[instrument(skip(self, llm, catalog))]
    pub async fn plan_from_intent(
        &self,
        created_by: Option<Uuid>,
        intent: &str,
        catalog: &str,
        llm: &dyn LlmClient,
    ) -> Result<Plan> {
        let prompt = format!(
            "CAPABILITY CATALOG\n{catalog}\n\nINTENT\n{intent}\n\nReturn the JSON plan."
        );
        let req = CompletionRequest {
            model: String::new(),
            messages: vec![ChatMessage::user(prompt)],
            system: Some(PLANNER_SYSTEM_PROMPT.to_string()),
            max_tokens: 2048,
            temperature: Some(0.0),
            tools: Vec::new(),
            request_id: None,
        };
        let resp = llm
            .complete(req)
            .await
            .map_err(|e| Error::Dispatch(format!("llm: {e}")))?;
        let text = resp
            .content
            .iter()
            .filter_map(|c| match c {
                daimon_llm::AssistantContent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();

        // Strip code fences if the model wrapped the JSON.
        let cleaned = text
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        #[derive(serde::Deserialize)]
        struct PlanShape {
            steps: Vec<StepDefShape>,
        }
        #[derive(serde::Deserialize)]
        struct StepDefShape {
            capability_name: String,
            capability_version: String,
            #[serde(default)]
            target_ref: Option<String>,
            #[serde(default)]
            credential_ref: Option<String>,
            #[serde(default)]
            params: Json,
            #[serde(default)]
            depends_on_index: Vec<usize>,
        }

        let parsed: PlanShape = serde_json::from_str(cleaned)
            .map_err(|e| Error::Decode(format!("LLM plan JSON: {e} (body: {cleaned})")))?;
        let steps = parsed
            .steps
            .into_iter()
            .map(|s| StepDef {
                capability_name: s.capability_name,
                capability_version: s.capability_version,
                target_ref: s.target_ref,
                credential_ref: s.credential_ref,
                params: s.params,
                depends_on_index: s.depends_on_index,
            })
            .collect();

        let plan = self.create_plan(created_by, intent, steps).await?;
        info!(plan_id = %plan.id, "LLM-emitted plan persisted");
        Ok(plan)
    }

    pub async fn list_plans(&self, limit: i64) -> Result<Vec<Plan>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, created_by, intent, status, metadata,
                        created_at, updated_at, started_at, finished_at
                 FROM public.plans
                 ORDER BY created_at DESC
                 LIMIT $1",
                &[&limit],
            )
            .await?;
        Ok(rows.into_iter().map(row_to_plan).collect())
    }

    pub async fn get_plan(&self, id: Uuid) -> Result<Option<Plan>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "SELECT id, created_by, intent, status, metadata,
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
        let mut done: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
        // Succeeded steps in the order they ran — the reverse-walk order for
        // the saga compensation pass (P2 commit 7).
        let mut succeeded_order: Vec<usize> = Vec::new();
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
                    succeeded_order.push(idx);
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

        // P2 commit 7 — saga rollback. On fail-stop, walk the SUCCEEDED steps in
        // REVERSE and compensate each one that declares a compensating
        // capability. Read-only / irreversible / uncompensated steps are
        // skipped and left visible for manual remediation. Compensating
        // dispatches go through the SAME bus → driver → broker → Guard path (a
        // compensating write is still gated).
        if failed {
            for &idx in succeeded_order.iter().rev() {
                let step = steps[idx].clone();
                match self.compensate_step(&step, actor_id).await {
                    Ok(true) => {
                        steps[idx].status = StepStatus::Compensated;
                    }
                    Ok(false) => { /* no compensator — leave Succeeded */ }
                    Err(e) => {
                        // A failed compensation is logged but does not abort the
                        // pass — the operator sees the un-compensated step and
                        // remediates manually.
                        error!(error = %e, step_id = %step.id, "compensation failed");
                    }
                }
            }
        }

        let final_status = if failed { PlanStatus::Failed } else { PlanStatus::Succeeded };
        self.finish_plan(plan_id, final_status).await?;
        Ok(final_status)
    }

    /// Compensate a single succeeded step (P2 commit 7).
    ///
    /// Looks up the step's `Capability` in the registry (via the dispatcher's
    /// registry — the SAME registry the forward dispatch resolves against) to
    /// read its `compensating` reference. Returns:
    /// - `Ok(false)` if the capability has no compensator (read-only /
    ///   irreversible) — nothing to do, the step is left as-is.
    /// - `Ok(true)` after the compensator has been dispatched over the bus and
    ///   the step marked `Compensated`.
    ///
    /// The compensating dispatch uses the `VersionReq` from
    /// `CompensatingCapability.version_req` (or `*` if `None`, per D18: pick the
    /// highest available), with params derived from the stored step result /
    /// Receipt. It runs through the same Guard gate as any write.
    async fn compensate_step(&self, step: &Step, actor_id: &str) -> Result<bool> {
        let _ = actor_id;
        // The dispatch + registry-resolution is pool-free (unit-testable with a
        // fake driver over an in-proc bus, see tests/saga.rs); only the
        // Compensated persistence needs the pool.
        match dispatch_compensator(&self.dispatcher, step).await? {
            Some(reply) => {
                self.set_step_status(step.id, StepStatus::Compensated, Some(reply), false)
                    .await?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Dispatch one plan step over the bus (P2 commit 6).
    ///
    /// The step names a `capability_name` + `capability_version` + typed
    /// `params`; there is NO raw-command path. `capability_version` is parsed
    /// into a `VersionReq` and resolution FAILS CLOSED on a bad/unsatisfiable
    /// string (FR-HAR-09) — no fallback to a version-incompatible agent, no
    /// fallback to a direct broker call. The body is `NetworkRequest`-shaped
    /// (capability + target_ref + typed params); the driver renders the device
    /// command from the params and dispatches through `broker.execute` (Guard
    /// gate, audit) — the same path a chat write takes. The reply is a
    /// serialized `NetworkResponse` returned verbatim as the step result.
    async fn dispatch_step(&self, step: &Step, actor_id: &str) -> Result<Json> {
        let version_req: VersionReq = step.capability_version.parse().map_err(|e| {
            Error::Dispatch(format!(
                "bad capability_version `{}` for step {}: {e}",
                step.capability_version, step.id
            ))
        })?;
        let target_ref = step
            .target_ref
            .clone()
            .ok_or_else(|| Error::Dispatch(format!("step {} has no target_ref", step.id)))?;

        let body = capability_body(&step.capability_name, &target_ref, &step.params);

        // FR-HAR-17: the audit/broker actor is the orchestrator step identity.
        // The originating operator is recorded on the plan row (created_by);
        // `actor_id` is threaded for the enqueue path but the bus `from` carries
        // the step-scoped id so audit shows which plan/step drove the write.
        let _ = actor_id;
        let from = AgentId::new(format!("orchestrator:{}:{}", step.plan_id, step.id));

        let reply = self
            .dispatcher
            .dispatch(
                from,
                &step.capability_name,
                &version_req,
                body,
                STEP_DISPATCH_TIMEOUT,
            )
            .await
            .map_err(|e| Error::Dispatch(e.to_string()))?;
        Ok(reply)
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
        // The status UPDATE is the terminal chokepoint + the TRUTH — it must
        // land (and the audit trail elsewhere is truth too). `RETURNING intent`
        // gives us the plan intent for the memory capture without a second
        // round-trip.
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                "UPDATE public.plans
                 SET status = $1, finished_at = now(), updated_at = now()
                 WHERE id = $2
                 RETURNING intent",
                &[&status.as_str(), &plan_id],
            )
            .await?;
        let intent: String = row.get(0);

        // P3 commit 10 — capture a typed record on a terminal state. AFTER the
        // status write (audit is truth, memory is the aid). Fail-soft: any
        // capture error logs and is swallowed — a memory fault must NEVER fail
        // the plan transition. Only Succeeded/Failed carry a record; other
        // terminal-ish states (Cancelled/RolledBack) are not captured here.
        if let Some(memory) = &self.memory {
            if let Some(record) = terminal_record(status, plan_id, &intent) {
                match memory.capture(record).await {
                    Ok(uri) => {
                        info!(plan_id = %plan_id, uri = %uri, "terminal plan record captured")
                    }
                    Err(e) => warn!(
                        plan_id = %plan_id,
                        error = %e,
                        "terminal plan record capture failed (fail-soft — ignored)"
                    ),
                }
            }
        }
        Ok(())
    }
}

/// The typed record a terminal plan state captures (P3 commit 10). Pure (no
/// I/O) so the Succeeded→Decision / Failed→Incident mapping is unit-testable
/// without a live pool or memory backend. `None` for any non-Succeeded/Failed
/// status (Cancelled/RolledBack/etc. are not captured here).
fn terminal_record(status: PlanStatus, plan_id: Uuid, intent: &str) -> Option<TypedRecord> {
    let body = match status {
        PlanStatus::Succeeded => TypedBody::Decision {
            title: format!("plan completed: {intent}"),
            context: intent.to_string(),
            decision: "plan completed".to_string(),
            rationale: format!("plan {plan_id} reached terminal state Succeeded"),
        },
        PlanStatus::Failed => TypedBody::Incident {
            title: format!("plan failed: {intent}"),
            impact: intent.to_string(),
            resolution: format!(
                "plan {plan_id} failed / rolled back (compensation pass ran for \
                 succeeded write steps)"
            ),
        },
        _ => return None,
    };
    Some(TypedRecord { body, namespace: None })
}

/// Resolve a step's compensator from the registry and dispatch it over the bus
/// (P2 commit 7). Pool-free so it is unit-testable with a fake driver over an
/// in-proc bus (the caller persists `Compensated`).
///
/// Returns:
/// - `Ok(None)` if the step's capability has no compensator (read-only /
///   irreversible / unregistered) — nothing dispatched.
/// - `Ok(Some(reply))` after the compensator has been dispatched and its
///   `NetworkResponse`-shaped reply returned (to store as the step result).
///
/// The compensating `VersionReq` comes from `CompensatingCapability.version_req`
/// (or `*` if `None`, D18: pick the highest available). Params are derived from
/// the original step params + stored Receipt via [`compensation_params`]. The
/// dispatch runs through the SAME bus → driver → broker → Guard path as any
/// write, so a compensating write is still gated.
async fn dispatch_compensator(dispatcher: &Dispatcher, step: &Step) -> Result<Option<Json>> {
    let Some(comp) = compensator_for(dispatcher, &step.capability_name).await else {
        return Ok(None);
    };
    let comp_req: VersionReq = match comp.version_req.as_deref() {
        Some(s) => s
            .parse()
            .map_err(|e| Error::Dispatch(format!("bad compensating version_req `{s}`: {e}")))?,
        None => "*".parse().expect("`*` is a valid VersionReq"),
    };
    let target_ref = step
        .target_ref
        .clone()
        .ok_or_else(|| Error::Dispatch(format!("step {} has no target_ref", step.id)))?;

    let params = compensation_params(&step.params, step.result.as_ref());
    let body = capability_body(&comp.name, &target_ref, &params);

    let from = AgentId::new(format!("orchestrator:{}:{}:compensate", step.plan_id, step.id));
    let reply = dispatcher
        .dispatch(from, &comp.name, &comp_req, body, STEP_DISPATCH_TIMEOUT)
        .await
        .map_err(|e| Error::Dispatch(e.to_string()))?;
    info!(step_id = %step.id, compensator = %comp.name, "step compensated");
    Ok(Some(reply))
}

/// Resolve the compensating reference for a capability from the registry.
/// `None` if the capability is unregistered or declares no compensator. Matches
/// on name only (any version) — the compensator is a property of the capability
/// contract, not a specific version pin.
async fn compensator_for(
    dispatcher: &Dispatcher,
    capability_name: &str,
) -> Option<daimon_core::CompensatingCapability> {
    let any: VersionReq = "*".parse().ok()?;
    let entry = dispatcher
        .registry()
        .require_by_capability(capability_name, &any)
        .await
        .ok()?;
    entry
        .capabilities
        .into_iter()
        .find(|c| c.name == capability_name)
        .and_then(|c| c.compensating)
}

/// Build the `NetworkRequest`-shaped dispatch body for a capability call
/// (P2 commit 6). Shape: `{capability, target_ref, timeout_secs?, params}` —
/// the wire contract the driver's bus adapter decodes. Built as raw JSON (not
/// via the driver's `NetworkRequest` type) so the orchestrator stays
/// driver-agnostic and does not depend on any specific driver crate (D21).
fn capability_body(capability: &str, target_ref: &str, params: &Json) -> Json {
    let timeout_secs = params
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(30);
    json!({
        "capability": capability,
        "target_ref": target_ref,
        "timeout_secs": timeout_secs,
        "params": params,
    })
}

/// Derive the parameters for a compensating dispatch from the ORIGINAL step
/// params + the stored success result (Receipt) (SDS §4.9.1).
///
/// Default: the compensator is keyed on the original params (idempotent inverse
/// on the same inputs). Special case for the RouterOS firewall pair
/// (`firewall_add_drop_rule` → `firewall_remove_rule`): the `comment` used on
/// add is the natural selector for remove, so it is projected onto `match` when
/// the original params carry a `comment` and no explicit `match` (SDS §4.9.1
/// worked example). The `_result` receipt is accepted for future use (e.g. a
/// row id emitted by the write); today the comment→match projection needs only
/// the original params.
fn compensation_params(original: &Json, _result: Option<&Json>) -> Json {
    let mut params = original.clone();
    if let Some(obj) = params.as_object_mut() {
        if !obj.contains_key("match") {
            if let Some(comment) = obj.get("comment").and_then(|v| v.as_str()) {
                obj.insert("match".to_string(), Json::String(comment.to_string()));
            }
        }
    }
    params
}

fn row_to_plan(row: tokio_postgres::Row) -> Plan {
    let status_str: String = row.get(3);
    Plan {
        id: row.get(0),
        created_by: row.get(1),
        intent: row.get(2),
        status: PlanStatus::from_str(&status_str),
        metadata: row.get(4),
        created_at: row.get(5),
        updated_at: row.get(6),
        started_at: row.get(7),
        finished_at: row.get(8),
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

#[cfg(test)]
mod saga_tests {
    //! Saga compensation unit tests (P2 commit 7).
    //!
    //! `run_plan` itself needs a live Postgres pool (covered by the `#[ignore]`
    //! graph_mirror integration test), but the compensation MECHANISM — resolve
    //! the compensator from the registry, dispatch it over the bus, get the
    //! reply — is pool-free (`dispatch_compensator`). These tests exercise it
    //! with a fake driver agent over a real in-proc `Dispatcher`, modelling the
    //! run_plan scenario: step 1 (a write with a compensator) succeeded, step 2
    //! failed → step 1's compensator is dispatched to the driver.

    use super::*;
    use async_trait::async_trait;
    use daimon_core::{
        Agent, AgentContext, AgentEnvelope, Capability, CompensatingCapability, CoreError,
    };
    use daimon_runtime::{CapabilityRegistry, InProcBus, Supervisor};
    use semver::Version;
    use std::sync::{Arc, Mutex};

    /// A fake firewall driver: advertises a write cap (`test.fw.add`) whose
    /// compensator is `test.fw.remove`, plus the compensator cap itself. Records
    /// every capability it is asked to run (via the bus body) and replies with a
    /// `NetworkResponse`-shaped success so the caller's decode path is exercised.
    struct FakeFirewallDriver {
        id: AgentId,
        caps: Vec<Capability>,
        seen: Arc<Mutex<Vec<(String, Json)>>>,
    }

    impl FakeFirewallDriver {
        fn new(seen: Arc<Mutex<Vec<(String, Json)>>>) -> Self {
            let add = Capability {
                name: "test.fw.add".into(),
                version: Version::new(1, 0, 0),
                description: Some("add drop rule".into()),
                schema: None,
                compensating: Some(CompensatingCapability {
                    name: "test.fw.remove".into(),
                    version_req: None, // → dispatched with "*"
                }),
                irreversible: false,
            };
            let remove = Capability {
                name: "test.fw.remove".into(),
                version: Version::new(1, 0, 0),
                description: Some("remove rule".into()),
                schema: None,
                compensating: None,
                irreversible: true,
            };
            Self {
                id: AgentId::new("fake-fw"),
                caps: vec![add, remove],
                seen,
            }
        }
    }

    #[async_trait]
    impl Agent for FakeFirewallDriver {
        fn id(&self) -> &AgentId {
            &self.id
        }
        fn capabilities(&self) -> &[Capability] {
            &self.caps
        }
        async fn handle(
            &self,
            env: AgentEnvelope,
            ctx: AgentContext,
        ) -> std::result::Result<(), CoreError> {
            let cap = env
                .body
                .get("capability")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let params = env.body.get("params").cloned().unwrap_or(Json::Null);
            self.seen.lock().unwrap().push((cap.clone(), params));
            // NetworkResponse-shaped success reply.
            let reply_body = json!({
                "success": true,
                "output": {
                    "command": format!("ran {cap}"),
                    "stdout": "",
                    "stderr": "",
                    "exit_status": 0
                },
                "error": null
            });
            let reply = AgentEnvelope::reply_to(&env, self.id.clone(), reply_body);
            ctx.bus.send(reply).await
        }
    }

    async fn dispatcher_with_fake_fw(
        seen: Arc<Mutex<Vec<(String, Json)>>>,
    ) -> (Dispatcher, Arc<Supervisor>) {
        let bus = InProcBus::new();
        let registry = CapabilityRegistry::new();
        let supervisor = Arc::new(Supervisor::new(bus.clone(), registry.clone()));
        supervisor
            .spawn(Arc::new(FakeFirewallDriver::new(seen)) as Arc<dyn Agent>)
            .await
            .expect("spawn fake fw");
        tokio::time::sleep(Duration::from_millis(100)).await;
        (Dispatcher::new(bus, registry), supervisor)
    }

    fn succeeded_write_step() -> Step {
        // Models step 1 after it succeeded: a `test.fw.add` write with a
        // `comment` (which compensation_params projects onto `match` for the
        // remove compensator), and a stored NetworkResponse-shaped receipt.
        Step {
            id: Uuid::new_v4(),
            plan_id: Uuid::new_v4(),
            step_index: 0,
            capability_name: "test.fw.add".into(),
            capability_version: "1.0.0".into(),
            target_ref: Some("target://edge".into()),
            credential_ref: None,
            params: json!({ "dst_address": "10.0.0.0/24", "comment": "saga-marker" }),
            depends_on: vec![],
            compensating_step_id: None,
            status: StepStatus::Succeeded,
            result: Some(json!({ "success": true })),
            started_at: None,
            finished_at: None,
        }
    }

    #[tokio::test]
    async fn compensator_is_dispatched_for_succeeded_write() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (dispatcher, _sup) = dispatcher_with_fake_fw(seen.clone()).await;

        let step = succeeded_write_step();
        let out = dispatch_compensator(&dispatcher, &step)
            .await
            .expect("dispatch ok");

        // A reply came back (the step gets marked Compensated in run_plan).
        assert!(out.is_some(), "compensator must have been dispatched");
        assert_eq!(out.unwrap()["success"], true);

        // The driver was asked to run the COMPENSATOR (`test.fw.remove`), with
        // `match` projected from the original `comment`.
        let calls = seen.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "exactly one compensating dispatch");
        assert_eq!(calls[0].0, "test.fw.remove");
        assert_eq!(calls[0].1["match"], "saga-marker");
    }

    #[tokio::test]
    async fn read_only_step_has_no_compensator_and_is_skipped() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (dispatcher, _sup) = dispatcher_with_fake_fw(seen.clone()).await;

        // `test.fw.remove` declares no compensator (irreversible) → skip.
        let mut step = succeeded_write_step();
        step.capability_name = "test.fw.remove".into();
        let out = dispatch_compensator(&dispatcher, &step)
            .await
            .expect("dispatch ok");
        assert!(out.is_none(), "no compensator → nothing dispatched");
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn compensation_params_projects_comment_onto_match() {
        let original = json!({ "dst_address": "10.0.0.0/24", "comment": "blk" });
        let p = compensation_params(&original, None);
        assert_eq!(p["match"], "blk");
        // Original keys preserved.
        assert_eq!(p["dst_address"], "10.0.0.0/24");
    }

    #[test]
    fn compensation_params_keeps_explicit_match() {
        let original = json!({ "match": "rule-7", "comment": "ignored" });
        let p = compensation_params(&original, None);
        assert_eq!(p["match"], "rule-7");
    }

    #[test]
    fn capability_body_has_network_request_shape() {
        let body = capability_body("test.fw.add", "target://edge", &json!({ "a": 1 }));
        assert_eq!(body["capability"], "test.fw.add");
        assert_eq!(body["target_ref"], "target://edge");
        assert_eq!(body["timeout_secs"], 30);
        assert_eq!(body["params"]["a"], 1);
    }
}

#[cfg(test)]
mod capture_tests {
    //! P3 commit 10 — terminal-state typed-record capture.
    //!
    //! `finish_plan` needs a live pool, so these pool-free units cover the
    //! LOAD-BEARING logic directly: the Succeeded→Decision / Failed→Incident /
    //! other→None mapping (`terminal_record`) and the fail-soft capture contract
    //! (a memory backend that errors must NOT propagate). The full
    //! finish_plan-persists-then-captures path is exercised by the DB-gated
    //! integration coverage.

    use super::*;
    use async_trait::async_trait;
    use daimon_memory::{
        IngestDoc, IngestStats, MemoryHealth, PreTurnContext, RecallBudget, RecordKind,
        RetrieveQuery, RetrievedChunk, ScoredRecord,
    };
    use std::sync::Mutex as StdMutex;

    #[test]
    fn succeeded_maps_to_decision() {
        let id = Uuid::new_v4();
        let rec = terminal_record(PlanStatus::Succeeded, id, "block 1.2.3.4")
            .expect("Succeeded captures a record");
        assert_eq!(rec.body.kind(), RecordKind::Decision);
        match rec.body {
            TypedBody::Decision { context, decision, rationale, .. } => {
                assert_eq!(context, "block 1.2.3.4");
                assert_eq!(decision, "plan completed");
                assert!(rationale.contains(&id.to_string()));
            }
            other => panic!("expected Decision, got {other:?}"),
        }
    }

    #[test]
    fn failed_maps_to_incident() {
        let id = Uuid::new_v4();
        let rec = terminal_record(PlanStatus::Failed, id, "block 1.2.3.4")
            .expect("Failed captures a record");
        assert_eq!(rec.body.kind(), RecordKind::Incident);
        match rec.body {
            TypedBody::Incident { impact, resolution, .. } => {
                assert_eq!(impact, "block 1.2.3.4");
                assert!(resolution.contains("failed"));
                assert!(resolution.contains(&id.to_string()));
            }
            other => panic!("expected Incident, got {other:?}"),
        }
    }

    #[test]
    fn non_terminal_statuses_capture_nothing() {
        let id = Uuid::new_v4();
        for st in [
            PlanStatus::Planning,
            PlanStatus::AwaitingApproval,
            PlanStatus::Executing,
            PlanStatus::Cancelled,
            PlanStatus::RolledBack,
        ] {
            assert!(terminal_record(st, id, "x").is_none(), "{st:?} → no record");
        }
    }

    /// A memory stub that records captures and can be forced to fail.
    struct StubMemory {
        captured: StdMutex<Vec<RecordKind>>,
        fail: bool,
    }

    #[async_trait]
    impl MemoryService for StubMemory {
        async fn ingest(&self, doc: IngestDoc) -> daimon_memory::Result<IngestStats> {
            Ok(IngestStats { source_id: doc.source_id, chunks: 1, collection: "t".into() })
        }
        async fn delete(&self, _uri: &str) -> daimon_memory::Result<()> {
            Ok(())
        }
        async fn retrieve(&self, _q: &RetrieveQuery) -> daimon_memory::Result<Vec<RetrievedChunk>> {
            Ok(vec![])
        }
        async fn capture(&self, rec: TypedRecord) -> daimon_memory::Result<String> {
            self.captured.lock().unwrap().push(rec.body.kind());
            if self.fail {
                Err(daimon_memory::Error::Unreachable("forced".into()))
            } else {
                Ok("daimon://plan/stub".into())
            }
        }
        async fn recall(&self, _q: &str, _b: RecallBudget) -> daimon_memory::Result<Vec<ScoredRecord>> {
            Ok(vec![])
        }
        async fn pre_turn_recall(&self, _m: &str, _b: RecallBudget) -> PreTurnContext {
            PreTurnContext::degraded()
        }
        async fn health(&self) -> MemoryHealth {
            MemoryHealth::default()
        }
    }

    /// Mirrors finish_plan's capture branch: build the terminal record and
    /// capture it fail-soft. Proves a capture error is swallowed (returns Ok).
    async fn capture_fail_soft(memory: &dyn MemoryService, status: PlanStatus) -> Result<()> {
        if let Some(record) = terminal_record(status, Uuid::new_v4(), "intent") {
            if let Err(e) = memory.capture(record).await {
                // swallow — exactly what finish_plan does.
                tracing::warn!(error = %e, "capture failed (fail-soft)");
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn capture_is_attempted_on_success() {
        let mem = StubMemory { captured: StdMutex::new(vec![]), fail: false };
        capture_fail_soft(&mem, PlanStatus::Succeeded).await.unwrap();
        assert_eq!(mem.captured.lock().unwrap().as_slice(), &[RecordKind::Decision]);
    }

    #[tokio::test]
    async fn capture_error_is_swallowed_fail_soft() {
        let mem = StubMemory { captured: StdMutex::new(vec![]), fail: true };
        // Despite the forced capture error, this returns Ok (never propagates).
        let out = capture_fail_soft(&mem, PlanStatus::Failed).await;
        assert!(out.is_ok(), "a memory fault must not fail the plan transition");
        assert_eq!(mem.captured.lock().unwrap().as_slice(), &[RecordKind::Incident]);
    }
}
