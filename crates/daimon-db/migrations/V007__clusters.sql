-- V007 — public.clusters (PVE registry) + supporting app tables.
--
-- The daimon-app SQLite `daimon.db` currently holds: users (replaced by
-- public.users in V002), sessions, config, clusters, user_preferences.
-- Of those, public.clusters is the operator-facing one; the rest are
-- recreated here as public.* tables so the app can swap its rusqlite
-- access for tokio-postgres in D3b.
--
-- Per plans/2026-05-23-phase-2c-compliance-posture-plan.md D3.

CREATE TABLE public.clusters (
    id              TEXT         PRIMARY KEY,
    tenant_id       UUID         NOT NULL REFERENCES public.tenants(id) ON DELETE RESTRICT,
    name            TEXT         NOT NULL,
    api_url         TEXT         NOT NULL,
    token           TEXT         NOT NULL,
    notes           TEXT         NOT NULL DEFAULT '',
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT now()
);

COMMENT ON TABLE public.clusters IS 'PVE cluster registry. token is a PVE API token (api_url root, token id + secret).';

CREATE UNIQUE INDEX clusters_tenant_name_idx ON public.clusters(tenant_id, name);
CREATE INDEX clusters_tenant_idx ON public.clusters(tenant_id);

CREATE TRIGGER clusters_touch_updated_at
    BEFORE UPDATE ON public.clusters
    FOR EACH ROW EXECUTE FUNCTION public.touch_updated_at();

CREATE TABLE public.sessions (
    id              TEXT         PRIMARY KEY,
    user_id         UUID         NOT NULL REFERENCES public.users(id) ON DELETE CASCADE,
    expires_at      TIMESTAMPTZ  NOT NULL,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT now()
);

COMMENT ON TABLE public.sessions IS 'Active login sessions. Cleaned up by app on expiry.';

CREATE INDEX sessions_user_idx ON public.sessions(user_id);
CREATE INDEX sessions_expires_idx ON public.sessions(expires_at);

CREATE TABLE public.app_config (
    key             TEXT         PRIMARY KEY,
    value           TEXT         NOT NULL,
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT now()
);

COMMENT ON TABLE public.app_config IS 'Cluster-wide app config (k/v). Tenant-scoped settings live on public.tenants.settings.';

CREATE TRIGGER app_config_touch_updated_at
    BEFORE UPDATE ON public.app_config
    FOR EACH ROW EXECUTE FUNCTION public.touch_updated_at();

CREATE TABLE public.user_preferences (
    user_id         UUID         NOT NULL REFERENCES public.users(id) ON DELETE CASCADE,
    key             TEXT         NOT NULL,
    value           TEXT         NOT NULL,
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, key)
);

COMMENT ON TABLE public.user_preferences IS 'Per-user UI preferences (theme, dashboard layout, etc.).';

CREATE TRIGGER user_preferences_touch_updated_at
    BEFORE UPDATE ON public.user_preferences
    FOR EACH ROW EXECUTE FUNCTION public.touch_updated_at();
