-- V024 — observer.named_queries (REVIVAL P0, SDS §14.6).
--
-- The NamedQueryLibrary (daimon-observer) was TOML-hardcoded and V015's
-- comment assumed a table that was never created. This adds it, born
-- single-org (no tenant_id / no RLS). It stores the operator-curated
-- PromQL/MetricsQL query set surfaced in the observer UI + used by
-- anomaly detectors.

CREATE TABLE observer.named_queries (
    id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT         NOT NULL UNIQUE,
    source      TEXT         NOT NULL DEFAULT 'prometheus',
    query       TEXT         NOT NULL,
    description TEXT,
    labels      JSONB        NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ  NOT NULL DEFAULT now()
);

COMMENT ON TABLE observer.named_queries IS 'Operator-curated named metric queries (single-org). Surfaced in the observer UI + used by anomaly detectors.';

CREATE INDEX named_queries_source_idx ON observer.named_queries(source);

CREATE TRIGGER named_queries_touch_updated_at
    BEFORE UPDATE ON observer.named_queries
    FOR EACH ROW EXECUTE FUNCTION public.touch_updated_at();
