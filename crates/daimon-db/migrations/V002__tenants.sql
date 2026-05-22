-- V002 — tenants, users, roles, role_grants + row-level security baseline.
--
-- Multi-tenant primitive. Every state-bearing table downstream (vault, inventory,
-- audit, plan_history, clusters) carries a `tenant_id` FK to public.tenants and
-- has an RLS policy enforcing it. Cluster-admin users have a `global` scope and
-- bypass tenant filtering for ops queries.
--
-- Per MASTERPLAN.md §4.4 and plans/2026-05-23-phase-2c-compliance-posture-plan.md D6.

CREATE EXTENSION IF NOT EXISTS pgcrypto;  -- for gen_random_uuid()

-- ---- tenants ----------------------------------------------------------------

CREATE TABLE public.tenants (
    id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    slug        TEXT         NOT NULL UNIQUE,
    name        TEXT         NOT NULL,
    status      TEXT         NOT NULL DEFAULT 'active',
    settings    JSONB        NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ  NOT NULL DEFAULT now(),
    CHECK (status IN ('active', 'suspended', 'archived'))
);

COMMENT ON TABLE public.tenants IS 'Tenant registry. Slug is the human-readable handle used in collection naming + audit target_ref.';
COMMENT ON COLUMN public.tenants.settings IS 'Per-tenant JSON config (LLM provider routing, retention overrides, etc.).';

CREATE INDEX tenants_status_idx ON public.tenants(status);

-- Seed the default dev tenant — matches the hard-coded `"default"` used by
-- daimon-rag's per-tenant collection naming in Phase 3.
INSERT INTO public.tenants (slug, name, status)
VALUES ('default', 'Default (dev)', 'active')
ON CONFLICT (slug) DO NOTHING;

-- ---- users ------------------------------------------------------------------

CREATE TABLE public.users (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID         REFERENCES public.tenants(id) ON DELETE RESTRICT,
    username        TEXT         NOT NULL,
    email           TEXT,
    password_hash   TEXT         NOT NULL,
    mfa_secret      TEXT,
    status          TEXT         NOT NULL DEFAULT 'active',
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT now(),
    last_login_at   TIMESTAMPTZ,
    CHECK (status IN ('active', 'disabled', 'locked'))
);

COMMENT ON TABLE public.users IS 'Operator accounts. tenant_id NULL = cluster_admin scope (cross-tenant).';
COMMENT ON COLUMN public.users.mfa_secret IS 'TOTP secret for MFA enrolment. NULL = MFA not enrolled.';

CREATE UNIQUE INDEX users_tenant_username_idx
    ON public.users(COALESCE(tenant_id, '00000000-0000-0000-0000-000000000000'::uuid), username);
CREATE INDEX users_tenant_idx ON public.users(tenant_id);
CREATE INDEX users_status_idx ON public.users(status);

-- ---- roles ------------------------------------------------------------------

CREATE TABLE public.roles (
    id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    slug        TEXT         NOT NULL UNIQUE,
    name        TEXT         NOT NULL,
    description TEXT,
    is_system   BOOLEAN      NOT NULL DEFAULT false,
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT now()
);

COMMENT ON TABLE public.roles IS 'RBAC role catalogue. is_system roles cannot be deleted.';

INSERT INTO public.roles (slug, name, description, is_system) VALUES
    ('cluster_admin', 'Cluster Administrator', 'Global scope — manages all tenants and cluster-level config.', true),
    ('tenant_admin', 'Tenant Administrator', 'Per-tenant administration — credentials, targets, audit, users.', true),
    ('operator', 'Operator', 'Runs agents, approves plans, reads audit.', true),
    ('viewer', 'Viewer', 'Read-only access to dashboards and audit.', true),
    ('auditor', 'Auditor', 'Read-only access to audit log + tamper-evidence verification.', true)
ON CONFLICT (slug) DO NOTHING;

-- ---- role_grants ------------------------------------------------------------

CREATE TABLE public.role_grants (
    id           UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID         NOT NULL REFERENCES public.users(id) ON DELETE CASCADE,
    role_id      UUID         NOT NULL REFERENCES public.roles(id) ON DELETE RESTRICT,
    -- Scope: 'global' or 'tenant:<uuid>'. cluster_admin role must use 'global';
    -- other roles must use 'tenant:<uuid>' matching the user's tenant_id.
    scope        TEXT         NOT NULL,
    granted_at   TIMESTAMPTZ  NOT NULL DEFAULT now(),
    granted_by   UUID         REFERENCES public.users(id) ON DELETE SET NULL,
    UNIQUE (user_id, role_id, scope)
);

COMMENT ON TABLE public.role_grants IS 'User <-> role <-> scope. Scope ''global'' is cluster-wide; ''tenant:<uuid>'' is tenant-scoped.';
COMMENT ON COLUMN public.role_grants.granted_by IS 'NULL = system-granted (e.g. initial cluster_admin bootstrap).';

CREATE INDEX role_grants_user_idx ON public.role_grants(user_id);
CREATE INDEX role_grants_role_idx ON public.role_grants(role_id);
CREATE INDEX role_grants_scope_idx ON public.role_grants(scope);

-- ---- updated_at trigger -----------------------------------------------------

CREATE OR REPLACE FUNCTION public.touch_updated_at() RETURNS trigger AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER tenants_touch_updated_at
    BEFORE UPDATE ON public.tenants
    FOR EACH ROW EXECUTE FUNCTION public.touch_updated_at();

CREATE TRIGGER users_touch_updated_at
    BEFORE UPDATE ON public.users
    FOR EACH ROW EXECUTE FUNCTION public.touch_updated_at();
