-- V013 — Guard tier (Phase 5): approval inbox.
--
-- Operator approval workflow for Guard-gated write capabilities. When the
-- policy engine returns `require_approval`, the broker creates a row here
-- and polls until status flips to approved | denied | expired. Operators
-- decide via the /admin/approvals UI.
--
-- The policy DSL itself lives in TOML on disk (loaded at boot by
-- daimon-guard::PolicyEngine). Phase 5.1 may add a `public.policy_rules`
-- table for dynamic policy edits.

CREATE TABLE public.approvals (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID         NOT NULL REFERENCES public.tenants(id) ON DELETE RESTRICT,
    actor_id        TEXT         NOT NULL,
    capability      TEXT         NOT NULL,
    target_ref      TEXT,
    params          JSONB        NOT NULL DEFAULT '{}'::jsonb,
    status          TEXT         NOT NULL DEFAULT 'pending',
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT now(),
    decided_at      TIMESTAMPTZ,
    decided_by      UUID         REFERENCES public.users(id) ON DELETE SET NULL,
    CHECK (status IN ('pending', 'approved', 'denied', 'expired'))
);

COMMENT ON TABLE public.approvals IS 'Guard approval inbox. Broker parks on rows here while polling for operator decision.';
COMMENT ON COLUMN public.approvals.capability IS 'Capability name being gated (e.g. "network.firewall.filter_add").';

CREATE INDEX approvals_tenant_status_idx ON public.approvals(tenant_id, status);
CREATE INDEX approvals_tenant_created_idx ON public.approvals(tenant_id, created_at DESC);

ALTER TABLE public.approvals ENABLE ROW LEVEL SECURITY;

CREATE POLICY approvals_tenant_select ON public.approvals
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

CREATE POLICY approvals_tenant_modify ON public.approvals
    FOR ALL USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    )
    WITH CHECK (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );
