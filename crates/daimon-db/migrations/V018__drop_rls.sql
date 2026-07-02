-- V018 — Remove Row-Level Security entirely (REVIVAL P0, FR-FND-08).
--
-- Multi-tenancy is OUT: daimon is a single-organization system. RLS was
-- the tenant-isolation mechanism (V011 + the per-migration policies added
-- by V012/V013/V014). It was never actually enforced (the app connects as
-- the table owner, which bypasses ENABLE-mode RLS), so this is a pure
-- removal with no behavioral change — isolation, where still wanted, is a
-- multi-user IAM concern handled in P1, not a storage-tier concern.
--
-- Order matters: drop every policy first, then disable RLS, then drop the
-- helper functions the policies referenced. observer.metrics is skipped —
-- V015 already dropped that table (DROP POLICY ... IF EXISTS still errors
-- if the *table* is missing, so it must not be named here). app_config was
-- already de-RLS'd in V017.

-- ---- vault.credentials ----
DROP POLICY IF EXISTS credentials_tenant_select ON vault.credentials;
DROP POLICY IF EXISTS credentials_tenant_modify ON vault.credentials;
ALTER TABLE vault.credentials DISABLE ROW LEVEL SECURITY;

-- ---- inventory.targets ----
DROP POLICY IF EXISTS targets_tenant_select ON inventory.targets;
DROP POLICY IF EXISTS targets_tenant_modify ON inventory.targets;
ALTER TABLE inventory.targets DISABLE ROW LEVEL SECURITY;

-- ---- audit.events ----
DROP POLICY IF EXISTS events_tenant_select ON audit.events;
DROP POLICY IF EXISTS events_tenant_insert ON audit.events;
ALTER TABLE audit.events DISABLE ROW LEVEL SECURITY;

-- ---- audit.anchors ----
DROP POLICY IF EXISTS anchors_tenant_select ON audit.anchors;
DROP POLICY IF EXISTS anchors_tenant_insert ON audit.anchors;
ALTER TABLE audit.anchors DISABLE ROW LEVEL SECURITY;

-- ---- public.clusters (table itself dropped in V021) ----
DROP POLICY IF EXISTS clusters_tenant_select ON public.clusters;
DROP POLICY IF EXISTS clusters_tenant_modify ON public.clusters;
ALTER TABLE public.clusters DISABLE ROW LEVEL SECURITY;

-- ---- public.plans ----
DROP POLICY IF EXISTS plans_tenant_select ON public.plans;
DROP POLICY IF EXISTS plans_tenant_modify ON public.plans;
ALTER TABLE public.plans DISABLE ROW LEVEL SECURITY;

-- ---- public.plan_steps ----
DROP POLICY IF EXISTS plan_steps_via_plan_tenant ON public.plan_steps;
ALTER TABLE public.plan_steps DISABLE ROW LEVEL SECURITY;

-- ---- public.tenants (table itself dropped in V023) ----
DROP POLICY IF EXISTS tenants_select ON public.tenants;
DROP POLICY IF EXISTS tenants_modify_cluster_admin ON public.tenants;
ALTER TABLE public.tenants DISABLE ROW LEVEL SECURITY;

-- ---- public.users (KEPT for IAM; only its RLS goes) ----
DROP POLICY IF EXISTS users_tenant_select ON public.users;
DROP POLICY IF EXISTS users_tenant_modify ON public.users;
ALTER TABLE public.users DISABLE ROW LEVEL SECURITY;

-- ---- public.roles (KEPT) ----
DROP POLICY IF EXISTS roles_select_all ON public.roles;
DROP POLICY IF EXISTS roles_modify_cluster_admin ON public.roles;
ALTER TABLE public.roles DISABLE ROW LEVEL SECURITY;

-- ---- public.role_grants (KEPT) ----
DROP POLICY IF EXISTS role_grants_via_user_tenant_select ON public.role_grants;
DROP POLICY IF EXISTS role_grants_via_user_tenant_modify ON public.role_grants;
ALTER TABLE public.role_grants DISABLE ROW LEVEL SECURITY;

-- ---- public.sessions (KEPT) ----
DROP POLICY IF EXISTS sessions_select ON public.sessions;
DROP POLICY IF EXISTS sessions_modify ON public.sessions;
ALTER TABLE public.sessions DISABLE ROW LEVEL SECURITY;

-- ---- public.user_preferences (KEPT) ----
DROP POLICY IF EXISTS user_preferences_via_user_tenant ON public.user_preferences;
ALTER TABLE public.user_preferences DISABLE ROW LEVEL SECURITY;

-- ---- memory.documents ----
DROP POLICY IF EXISTS documents_tenant_select ON memory.documents;
DROP POLICY IF EXISTS documents_tenant_modify ON memory.documents;
ALTER TABLE memory.documents DISABLE ROW LEVEL SECURITY;

-- ---- memory.document_chunks ----
DROP POLICY IF EXISTS chunks_tenant_select ON memory.document_chunks;
DROP POLICY IF EXISTS chunks_tenant_modify ON memory.document_chunks;
ALTER TABLE memory.document_chunks DISABLE ROW LEVEL SECURITY;

-- ---- observer.anomalies ----
DROP POLICY IF EXISTS anomalies_tenant_select ON observer.anomalies;
DROP POLICY IF EXISTS anomalies_tenant_modify ON observer.anomalies;
ALTER TABLE observer.anomalies DISABLE ROW LEVEL SECURITY;

-- ---- public.approvals ----
DROP POLICY IF EXISTS approvals_tenant_select ON public.approvals;
DROP POLICY IF EXISTS approvals_tenant_modify ON public.approvals;
ALTER TABLE public.approvals DISABLE ROW LEVEL SECURITY;

-- ---- helper functions (only referenced by the now-dropped policies) ----
DROP FUNCTION IF EXISTS public.current_tenant_id();
DROP FUNCTION IF EXISTS public.current_role_is_cluster_admin();
