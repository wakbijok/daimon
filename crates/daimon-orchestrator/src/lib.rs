//! Orchestrator — plan persistence + topological execution.
//!
//! Phase 6 D1 ships:
//! - `Plan` + `Step` types matching the V006 schema.
//! - `OrchestratorService::create_plan` — persists a hand-crafted plan.
//! - `OrchestratorService::run_plan` — executes steps in topological order,
//!   persisting per-step state transitions to `public.plan_steps`.
//! - `OrchestratorService::list_plans` / `get_plan` — read paths for the UI.
//!
//! Phase 6 D2 adds `plan_from_llm` (LLM-emitted DAGs). Phase 6.1 adds
//! saga rollback (D18 — compensating capability dispatch on failure).

pub mod error;
pub mod plan;
pub mod service;

pub use error::{Error, Result};
pub use plan::{Plan, PlanStatus, Step, StepDef, StepStatus};
pub use service::OrchestratorService;
