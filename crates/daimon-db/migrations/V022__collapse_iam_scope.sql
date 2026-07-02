-- V022 — Collapse IAM from tenant-scoped to single-org (REVIVAL P0).
--
-- public.users / roles / role_grants are KEPT — multi-user IAM is IN (ITSM
-- roles). This migration only removes the *tenant scoping*: the tenant_id
-- column on users and the 'global'|'tenant:<uuid>' scope dimension on
-- role_grants. Dropping users.tenant_id auto-drops its FK and the two
-- tenant indexes (incl. the COALESCE(tenant_id,…) unique hack); we replace
-- them with a plain org-wide unique username — which by construction closes
-- the C7 latent cross-tenant-login bug (two users could share a username
-- across tenants and login's global LIMIT 1 could pick the wrong one).
--
-- The role-slug remap (cluster_admin/tenant_admin -> admin) is P1's IAM
-- de-tenant work; here we only add the single-org roles and drop the scope.

-- ---- users: drop tenant scoping, org-wide unique username ----
ALTER TABLE public.users DROP COLUMN IF EXISTS tenant_id;   -- drops FK + users_tenant_*_idx
CREATE UNIQUE INDEX IF NOT EXISTS users_username_idx ON public.users(username);
COMMENT ON TABLE public.users IS 'Operator accounts (single-org). Username unique org-wide.';

-- ---- role_grants: drop the scope dimension ----
ALTER TABLE public.role_grants DROP COLUMN IF EXISTS scope;  -- drops UNIQUE(user_id,role_id,scope) + scope_idx
ALTER TABLE public.role_grants
    ADD CONSTRAINT role_grants_user_role_uniq UNIQUE (user_id, role_id);
COMMENT ON TABLE public.role_grants IS 'User <-> role grants (single-org; no tenant scope).';

-- ---- seed the single-org roles P1 maps onto ----
INSERT INTO public.roles (slug, name, description, is_system) VALUES
    ('admin',    'Administrator', 'Full administration — users, credentials, targets, connectors, settings, audit.', true),
    ('approver', 'Approver',      'Approves guard-gated plan steps in the approval inbox.', true)
ON CONFLICT (slug) DO NOTHING;
