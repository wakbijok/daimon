-- V016 — Generic app_config key/value store for the /admin/settings UI.
--
-- REVIVAL P0: sanctioned in-place edit (FR-FND-03). As originally written,
-- this migration did a bare `CREATE TABLE public.app_config` — but V007
-- already created that table (flat key/value shape), so V016 aborted with
-- "relation already exists" on EVERY fresh database. It therefore never
-- applied cleanly on the fresh path and has no committed fresh-path
-- checksum; making it idempotent is the one sanctioned exception to the
-- no-edit-historical-migrations rule. A dev box that limped past V016
-- needs a one-time refinery checksum reset (or re-provision).
--
-- The tenant-scoped "rich" shape below only materializes on a database
-- where app_config was somehow born rich; on the fresh (V007) timeline
-- the guarded block is a no-op. V017 converges BOTH timelines to the
-- final single-org shape: (key PK, value JSONB, is_secret, updated_at,
-- updated_by).
--
-- Key conventions (dotted path; one tab per top segment):
--   identity.jwt_ttl_secs
--   connections.postgres_url      (read-only display; env wins at boot)
--   connections.redis_url
--   connections.vm_url
--   connections.graph_url
--   connections.nats_url
--   connections.prometheus_url
--   llm.anthropic_key             (is_secret = true; value = vault://settings/anthropic_key)
--   llm.openai_key                (is_secret = true; value = vault://settings/openai_key)
--   llm.ollama_url
--   llm.default_model.orchestrator
--   llm.default_model.network
--   llm.default_model.chat
--   guard.approval_timeout_secs
--   guard.kill_file_path
--   guard.blast_radius_depth
--   observer.prom_poll_interval_secs
--   rag.embedding_model
--   rag.chunk_size_tokens
--   rag.chunk_overlap_tokens
--   rag.reranker_top_k
--   rag.context_budget_tokens
--   rag.cross_encoder_enabled
--   vault.kms_backend             (LocalFile | VaultTransit | AwsKms | Pkcs11)
--   vault.master_envelope_path
--   vault.dek_rotation_days
--   audit.anchor_cadence_minutes
--   audit.anchor_s3_target
--   update.channel                (stable | beta | main)
--   update.last_check_at          (timestamptz as ISO-8601 string in JSONB)
--   update.last_check_latest      (the last release tag we saw)

CREATE TABLE IF NOT EXISTS public.app_config (
    tenant_id   UUID         NOT NULL,
    key         TEXT         NOT NULL,
    value       JSONB        NOT NULL DEFAULT '{}'::jsonb,
    is_secret   BOOLEAN      NOT NULL DEFAULT FALSE,
    updated_at  TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_by  UUID         REFERENCES public.users(id) ON DELETE SET NULL,
    PRIMARY KEY (tenant_id, key)
);

COMMENT ON TABLE public.app_config IS
    'Settings key-value store. Secrets carry vault:// refs in value, not plaintext.';

-- The tenant-shaped index + RLS only apply when the table was born rich
-- (i.e. the CREATE above actually ran). On the V007 flat timeline the
-- tenant_id column does not exist and this whole block is a no-op;
-- V017 finishes the convergence either way.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name   = 'app_config'
          AND column_name  = 'tenant_id'
    ) THEN
        CREATE INDEX IF NOT EXISTS app_config_tenant_prefix_idx
            ON public.app_config (tenant_id, key text_pattern_ops);

        ALTER TABLE public.app_config ENABLE ROW LEVEL SECURITY;

        DROP POLICY IF EXISTS app_config_tenant_select ON public.app_config;
        CREATE POLICY app_config_tenant_select ON public.app_config
            FOR SELECT
            USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

        DROP POLICY IF EXISTS app_config_tenant_insert ON public.app_config;
        CREATE POLICY app_config_tenant_insert ON public.app_config
            FOR INSERT
            WITH CHECK (tenant_id = current_setting('app.tenant_id', true)::uuid);

        DROP POLICY IF EXISTS app_config_tenant_update ON public.app_config;
        CREATE POLICY app_config_tenant_update ON public.app_config
            FOR UPDATE
            USING (tenant_id = current_setting('app.tenant_id', true)::uuid)
            WITH CHECK (tenant_id = current_setting('app.tenant_id', true)::uuid);

        DROP POLICY IF EXISTS app_config_tenant_delete ON public.app_config;
        CREATE POLICY app_config_tenant_delete ON public.app_config
            FOR DELETE
            USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
    END IF;
END $$;
