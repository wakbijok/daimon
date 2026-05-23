-- V006 — public.plans + public.plan_steps.
--
-- Base shape for Phase 6 Orchestrator DAG persistence. We ship the
-- skeleton now so Phase 2c's data layer is forward-compatible. Phase 6
-- will extend with replan history, saga rollback state, approval state.
--
-- Per MASTERPLAN.md §5.4 and plans/2026-05-23-phase-2c-compliance-posture-plan.md D2.

CREATE TABLE public.plans (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID         NOT NULL REFERENCES public.tenants(id) ON DELETE RESTRICT,
    created_by      UUID         REFERENCES public.users(id) ON DELETE SET NULL,
    intent          TEXT         NOT NULL,
    status          TEXT         NOT NULL DEFAULT 'planning',
    metadata        JSONB        NOT NULL DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT now(),
    started_at      TIMESTAMPTZ,
    finished_at     TIMESTAMPTZ,
    CHECK (status IN ('planning', 'awaiting_approval', 'executing', 'succeeded', 'failed', 'cancelled', 'rolled_back'))
);

COMMENT ON TABLE public.plans IS 'Orchestrator-emitted plan DAGs. One row per intent execution.';
COMMENT ON COLUMN public.plans.intent IS 'Free-form natural-language intent that produced the plan.';

CREATE INDEX plans_tenant_idx ON public.plans(tenant_id);
CREATE INDEX plans_tenant_status_idx ON public.plans(tenant_id, status);
CREATE INDEX plans_tenant_created_idx ON public.plans(tenant_id, created_at DESC);

CREATE TRIGGER plans_touch_updated_at
    BEFORE UPDATE ON public.plans
    FOR EACH ROW EXECUTE FUNCTION public.touch_updated_at();

CREATE TABLE public.plan_steps (
    id                      UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    plan_id                 UUID         NOT NULL REFERENCES public.plans(id) ON DELETE CASCADE,
    step_index              INTEGER      NOT NULL,
    capability_name         TEXT         NOT NULL,
    capability_version      TEXT         NOT NULL,
    target_ref              TEXT,
    credential_ref          TEXT,
    params                  JSONB        NOT NULL DEFAULT '{}'::jsonb,
    depends_on              UUID[]       NOT NULL DEFAULT ARRAY[]::UUID[],
    compensating_step_id    UUID,
    status                  TEXT         NOT NULL DEFAULT 'pending',
    result                  JSONB,
    started_at              TIMESTAMPTZ,
    finished_at             TIMESTAMPTZ,
    CHECK (status IN ('pending', 'running', 'succeeded', 'failed', 'skipped', 'compensated'))
);

COMMENT ON TABLE public.plan_steps IS 'Per-step state for a plan DAG. depends_on holds the prerequisite step ids.';
COMMENT ON COLUMN public.plan_steps.compensating_step_id IS 'For saga rollback (D18): the step that undoes this one when downstream fails.';

CREATE INDEX plan_steps_plan_idx ON public.plan_steps(plan_id);
CREATE INDEX plan_steps_plan_index_idx ON public.plan_steps(plan_id, step_index);
CREATE INDEX plan_steps_status_idx ON public.plan_steps(status);
