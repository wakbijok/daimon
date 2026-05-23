-- V016 — Generic app_config key/value store for the /admin/settings UI.
--
-- Design: one row per (tenant_id, key). Values are JSONB so we don't
-- need a column per setting type. Secrets (LLM API keys, KMS envelope
-- paths) are NOT stored here — they get a vault:// ref and the
-- ref-string is what lives in `value`. The `is_secret` flag tells the
-- UI to render with mask + reveal-with-reauth.
--
-- Key conventions (dotted path; one tab per top segment):
--   identity.tenant_name
--   identity.jwt_ttl_secs
--   connections.postgres_url      (read-only display; env wins at boot)
--   connections.qdrant_url
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
--   observer.pve_poll_interval_secs
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

CREATE TABLE public.app_config (
    tenant_id   UUID         NOT NULL REFERENCES public.tenants(id) ON DELETE CASCADE,
    key         TEXT         NOT NULL,
    value       JSONB        NOT NULL DEFAULT '{}'::jsonb,
    is_secret   BOOLEAN      NOT NULL DEFAULT FALSE,
    updated_at  TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_by  UUID         REFERENCES public.users(id) ON DELETE SET NULL,
    PRIMARY KEY (tenant_id, key)
);

CREATE INDEX app_config_tenant_prefix_idx
    ON public.app_config (tenant_id, key text_pattern_ops);

COMMENT ON TABLE public.app_config IS
    'Settings key-value store. Secrets carry vault:// refs in value, not plaintext.';
COMMENT ON COLUMN public.app_config.is_secret IS
    'TRUE means value is a vault:// reference; UI masks and requires reauth to reveal.';

-- RLS — same pattern as the rest of the tenant-scoped tables (V011).
ALTER TABLE public.app_config ENABLE ROW LEVEL SECURITY;

CREATE POLICY app_config_tenant_select ON public.app_config
    FOR SELECT
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

CREATE POLICY app_config_tenant_insert ON public.app_config
    FOR INSERT
    WITH CHECK (tenant_id = current_setting('app.tenant_id', true)::uuid);

CREATE POLICY app_config_tenant_update ON public.app_config
    FOR UPDATE
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid)
    WITH CHECK (tenant_id = current_setting('app.tenant_id', true)::uuid);

CREATE POLICY app_config_tenant_delete ON public.app_config
    FOR DELETE
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
