-- V011 — row-level security policies on tenant-scoped tables.
--
-- Strategy: each tenant-scoped table gets `ENABLE ROW LEVEL SECURITY` plus
-- policies that compare the row's `tenant_id` against the session GUC
-- `app.tenant_id`. The cluster-admin escape hatch uses a separate session
-- GUC `app.role` = 'cluster_admin'.
--
-- Application code must set those GUCs at the start of each request /
-- transaction, e.g.:
--
--     SET LOCAL app.tenant_id = '<uuid>';
--     SET LOCAL app.role = 'tenant_admin';
--
-- A helper in `daimon-db` (`with_tenant_session`) wraps a closure in a
-- transaction that runs SET LOCAL before any query.
--
-- BYPASSRLS for the connection role is intentionally NOT used — the policy
-- IS the security boundary. Postgres superusers and the connection role's
-- owner can still see all rows; that is operator-level access and is
-- expected.
--
-- Per MASTERPLAN.md §4.4 and plans/2026-05-23-phase-2c-compliance-posture-plan.md D6.

-- Helper: parses the session GUC into a UUID, returning NULL when unset.
CREATE OR REPLACE FUNCTION public.current_tenant_id() RETURNS UUID AS $$
DECLARE
    s TEXT;
BEGIN
    s := current_setting('app.tenant_id', true);
    IF s IS NULL OR s = '' THEN
        RETURN NULL;
    END IF;
    RETURN s::uuid;
EXCEPTION WHEN others THEN
    RETURN NULL;
END;
$$ LANGUAGE plpgsql STABLE;

CREATE OR REPLACE FUNCTION public.current_role_is_cluster_admin() RETURNS BOOLEAN AS $$
DECLARE
    s TEXT;
BEGIN
    s := current_setting('app.role', true);
    RETURN s = 'cluster_admin';
END;
$$ LANGUAGE plpgsql STABLE;

-- ---- vault.credentials ------------------------------------------------------

ALTER TABLE vault.credentials ENABLE ROW LEVEL SECURITY;

CREATE POLICY credentials_tenant_select ON vault.credentials
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

CREATE POLICY credentials_tenant_modify ON vault.credentials
    FOR ALL USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    )
    WITH CHECK (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

-- ---- inventory.targets ------------------------------------------------------

ALTER TABLE inventory.targets ENABLE ROW LEVEL SECURITY;

CREATE POLICY targets_tenant_select ON inventory.targets
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

CREATE POLICY targets_tenant_modify ON inventory.targets
    FOR ALL USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    )
    WITH CHECK (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

-- ---- audit.events -----------------------------------------------------------

ALTER TABLE audit.events ENABLE ROW LEVEL SECURITY;

CREATE POLICY events_tenant_select ON audit.events
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

-- INSERT path (the broker writes audit events on every state change).
CREATE POLICY events_tenant_insert ON audit.events
    FOR INSERT WITH CHECK (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

-- UPDATE/DELETE remain blocked by the V005 trigger; no policy needed.

-- ---- audit.anchors ----------------------------------------------------------

ALTER TABLE audit.anchors ENABLE ROW LEVEL SECURITY;

CREATE POLICY anchors_tenant_select ON audit.anchors
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

CREATE POLICY anchors_tenant_insert ON audit.anchors
    FOR INSERT WITH CHECK (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

-- ---- public.clusters --------------------------------------------------------

ALTER TABLE public.clusters ENABLE ROW LEVEL SECURITY;

CREATE POLICY clusters_tenant_select ON public.clusters
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

CREATE POLICY clusters_tenant_modify ON public.clusters
    FOR ALL USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    )
    WITH CHECK (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

-- ---- public.plans + public.plan_steps ---------------------------------------

ALTER TABLE public.plans ENABLE ROW LEVEL SECURITY;

CREATE POLICY plans_tenant_select ON public.plans
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

CREATE POLICY plans_tenant_modify ON public.plans
    FOR ALL USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    )
    WITH CHECK (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

ALTER TABLE public.plan_steps ENABLE ROW LEVEL SECURITY;

CREATE POLICY plan_steps_via_plan_tenant ON public.plan_steps
    FOR ALL USING (
        public.current_role_is_cluster_admin()
        OR EXISTS (
            SELECT 1 FROM public.plans p
            WHERE p.id = plan_id
            AND p.tenant_id = public.current_tenant_id()
        )
    )
    WITH CHECK (
        public.current_role_is_cluster_admin()
        OR EXISTS (
            SELECT 1 FROM public.plans p
            WHERE p.id = plan_id
            AND p.tenant_id = public.current_tenant_id()
        )
    );

-- ---- public.tenants + public.users + public.roles + public.role_grants ------
--
-- Special handling — these tables are the multi-tenant primitives. Only
-- cluster_admin can see across; users see their own tenant.

ALTER TABLE public.tenants ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenants_select ON public.tenants
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR id = public.current_tenant_id()
    );

-- INSERT/UPDATE on tenants is cluster-admin-only.
CREATE POLICY tenants_modify_cluster_admin ON public.tenants
    FOR ALL USING (public.current_role_is_cluster_admin())
    WITH CHECK (public.current_role_is_cluster_admin());

ALTER TABLE public.users ENABLE ROW LEVEL SECURITY;

CREATE POLICY users_tenant_select ON public.users
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

CREATE POLICY users_tenant_modify ON public.users
    FOR ALL USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    )
    WITH CHECK (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

-- public.roles is global config; readable by all authenticated callers,
-- only cluster_admin can mutate.
ALTER TABLE public.roles ENABLE ROW LEVEL SECURITY;

CREATE POLICY roles_select_all ON public.roles
    FOR SELECT USING (true);

CREATE POLICY roles_modify_cluster_admin ON public.roles
    FOR ALL USING (public.current_role_is_cluster_admin())
    WITH CHECK (public.current_role_is_cluster_admin());

ALTER TABLE public.role_grants ENABLE ROW LEVEL SECURITY;

CREATE POLICY role_grants_via_user_tenant_select ON public.role_grants
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR EXISTS (
            SELECT 1 FROM public.users u
            WHERE u.id = user_id
            AND u.tenant_id = public.current_tenant_id()
        )
    );

CREATE POLICY role_grants_via_user_tenant_modify ON public.role_grants
    FOR ALL USING (
        public.current_role_is_cluster_admin()
        OR EXISTS (
            SELECT 1 FROM public.users u
            WHERE u.id = user_id
            AND u.tenant_id = public.current_tenant_id()
        )
    )
    WITH CHECK (
        public.current_role_is_cluster_admin()
        OR EXISTS (
            SELECT 1 FROM public.users u
            WHERE u.id = user_id
            AND u.tenant_id = public.current_tenant_id()
        )
    );

-- ---- public.sessions + public.user_preferences + public.app_config ----------
--
-- sessions: only the owning user (via role_grants->user_id linkage) should
-- see their own sessions. Cluster admin sees all.

ALTER TABLE public.sessions ENABLE ROW LEVEL SECURITY;

CREATE POLICY sessions_select ON public.sessions
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR EXISTS (
            SELECT 1 FROM public.users u
            WHERE u.id = user_id
            AND u.tenant_id = public.current_tenant_id()
        )
    );

CREATE POLICY sessions_modify ON public.sessions
    FOR ALL USING (
        public.current_role_is_cluster_admin()
        OR EXISTS (
            SELECT 1 FROM public.users u
            WHERE u.id = user_id
            AND u.tenant_id = public.current_tenant_id()
        )
    )
    WITH CHECK (
        public.current_role_is_cluster_admin()
        OR EXISTS (
            SELECT 1 FROM public.users u
            WHERE u.id = user_id
            AND u.tenant_id = public.current_tenant_id()
        )
    );

ALTER TABLE public.user_preferences ENABLE ROW LEVEL SECURITY;

CREATE POLICY user_preferences_via_user_tenant ON public.user_preferences
    FOR ALL USING (
        public.current_role_is_cluster_admin()
        OR EXISTS (
            SELECT 1 FROM public.users u
            WHERE u.id = user_id
            AND u.tenant_id = public.current_tenant_id()
        )
    )
    WITH CHECK (
        public.current_role_is_cluster_admin()
        OR EXISTS (
            SELECT 1 FROM public.users u
            WHERE u.id = user_id
            AND u.tenant_id = public.current_tenant_id()
        )
    );

-- app_config is intentionally NOT tenant-scoped (cluster-wide settings).
-- Only cluster_admin can mutate. Anyone authenticated can read.
ALTER TABLE public.app_config ENABLE ROW LEVEL SECURITY;

CREATE POLICY app_config_select_all ON public.app_config
    FOR SELECT USING (true);

CREATE POLICY app_config_modify_cluster_admin ON public.app_config
    FOR ALL USING (public.current_role_is_cluster_admin())
    WITH CHECK (public.current_role_is_cluster_admin());
