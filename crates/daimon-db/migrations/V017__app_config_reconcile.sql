-- V017 — Reconcile public.app_config to the final single-org shape.
--
-- REVIVAL P0 (FR-FND-03). Two timelines converge here:
--   * FRESH path: V007 created a flat (key PK, value TEXT, updated_at)
--     table; V011 enabled RLS on it with policies app_config_select_all
--     + app_config_modify_cluster_admin; V016's CREATE is now a no-op.
--   * RICH path (any DB where app_config was born tenant-shaped): V016's
--     four app_config_tenant_* policies + tenant PK + JSONB value.
--
-- This migration drops BOTH policy families, removes the tenant scoping,
-- and lands the single-org shape the revived /admin/settings reads:
--   (key PK, value JSONB, is_secret, updated_at, updated_by).
-- Everything is guarded (IF EXISTS / information_schema) so it is a
-- no-op on whichever pieces are already in the target state.

-- 1. Drop every policy that could exist on app_config (both families).
DROP POLICY IF EXISTS app_config_tenant_select        ON public.app_config;  -- V016
DROP POLICY IF EXISTS app_config_tenant_insert        ON public.app_config;  -- V016
DROP POLICY IF EXISTS app_config_tenant_update        ON public.app_config;  -- V016
DROP POLICY IF EXISTS app_config_tenant_delete        ON public.app_config;  -- V016
DROP POLICY IF EXISTS app_config_select_all           ON public.app_config;  -- V011:319
DROP POLICY IF EXISTS app_config_modify_cluster_admin ON public.app_config;  -- V011:322

ALTER TABLE public.app_config DISABLE ROW LEVEL SECURITY;

-- 2. Drop the tenant PK + column (rich timeline only; guarded).
ALTER TABLE public.app_config DROP CONSTRAINT IF EXISTS app_config_pkey;
ALTER TABLE public.app_config DROP COLUMN     IF EXISTS tenant_id;

-- 3. Add the single-org columns the flat (V007) timeline lacks.
ALTER TABLE public.app_config ADD COLUMN IF NOT EXISTS is_secret  BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE public.app_config ADD COLUMN IF NOT EXISTS updated_by UUID
    REFERENCES public.users(id) ON DELETE SET NULL;

-- 4. Coerce value TEXT -> JSONB (flat timeline stored TEXT). Existing flat
--    rows become JSON strings; the settings UI only reads keys it wrote.
DO $$
BEGIN
    IF (SELECT data_type FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'app_config' AND column_name = 'value') = 'text'
    THEN
        ALTER TABLE public.app_config ALTER COLUMN value TYPE JSONB USING to_jsonb(value);
    END IF;
END $$;

ALTER TABLE public.app_config ALTER COLUMN value SET DEFAULT '{}'::jsonb;

-- 5. Single-org primary key on `key` (guarded — flat timeline already has it).
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.table_constraints
        WHERE table_schema = 'public' AND table_name = 'app_config'
          AND constraint_type = 'PRIMARY KEY'
    ) THEN
        ALTER TABLE public.app_config ADD PRIMARY KEY (key);
    END IF;
END $$;

-- 6. Swap the tenant-prefixed index for a plain key-prefix index.
DROP INDEX IF EXISTS public.app_config_tenant_prefix_idx;
CREATE INDEX IF NOT EXISTS app_config_key_prefix_idx
    ON public.app_config (key text_pattern_ops);

COMMENT ON TABLE public.app_config IS
    'Single-org settings key-value store. Secrets carry vault:// refs in value, not plaintext.';
COMMENT ON COLUMN public.app_config.is_secret IS
    'TRUE means value is a vault:// reference; UI masks and requires reauth to reveal.';
