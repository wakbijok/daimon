-- V014 — Observer tier (#4 of the 5-DB storage architecture). Phase 7.
--
-- Time-series telemetry from the platform pollers and Prometheus pulls,
-- plus an anomaly event log the bus emitters write to.
--
-- macOS dev uses plain Postgres tables. Production Linux deploys load the
-- TimescaleDB extension and convert observer.metrics to a hypertable:
--
--   CREATE EXTENSION IF NOT EXISTS timescaledb;
--   SELECT create_hypertable('observer.metrics', 'ts', if_not_exists => TRUE);
--
-- Both paths share the same schema + RLS policies. The hypertable
-- conversion is non-destructive on the row data.

CREATE SCHEMA IF NOT EXISTS observer;

COMMENT ON SCHEMA observer IS 'Time-series telemetry tier (#4). Convertible to TimescaleDB hypertable in production.';

-- ---- observer.metrics -------------------------------------------------------

CREATE TABLE observer.metrics (
    ts          TIMESTAMPTZ      NOT NULL,
    tenant_id   UUID             NOT NULL REFERENCES public.tenants(id) ON DELETE RESTRICT,
    source      TEXT             NOT NULL,
    source_id   TEXT             NOT NULL,
    name        TEXT             NOT NULL,
    value       DOUBLE PRECISION NOT NULL,
    labels      JSONB            NOT NULL DEFAULT '{}'::jsonb
);

COMMENT ON TABLE observer.metrics IS 'Time-series metrics. Convert to TimescaleDB hypertable in production.';
COMMENT ON COLUMN observer.metrics.source IS '"pve" / "prometheus" / "agent" — what tier emitted the metric.';
COMMENT ON COLUMN observer.metrics.source_id IS 'Per-source identifier — cluster_id for pve, instance label for prometheus, agent id for agents.';
COMMENT ON COLUMN observer.metrics.name IS 'Dotted metric name, e.g. "pve.node.cpu_pct" or "agent.llm.input_tokens".';

CREATE INDEX metrics_tenant_ts_name_idx ON observer.metrics(tenant_id, ts DESC, name);
CREATE INDEX metrics_source_idx ON observer.metrics(source, source_id);
CREATE INDEX metrics_labels_gin_idx ON observer.metrics USING gin(labels);

ALTER TABLE observer.metrics ENABLE ROW LEVEL SECURITY;

CREATE POLICY metrics_tenant_select ON observer.metrics
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

CREATE POLICY metrics_tenant_insert ON observer.metrics
    FOR INSERT WITH CHECK (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

-- ---- observer.anomalies ------------------------------------------------------

CREATE TABLE observer.anomalies (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID         NOT NULL REFERENCES public.tenants(id) ON DELETE RESTRICT,
    detected_at     TIMESTAMPTZ  NOT NULL DEFAULT now(),
    source          TEXT         NOT NULL,
    source_id       TEXT         NOT NULL,
    severity        TEXT         NOT NULL,
    title           TEXT         NOT NULL,
    description     TEXT,
    metric_name     TEXT,
    metric_value    DOUBLE PRECISION,
    threshold       DOUBLE PRECISION,
    metadata        JSONB        NOT NULL DEFAULT '{}'::jsonb,
    resolved_at     TIMESTAMPTZ,
    CHECK (severity IN ('info', 'warning', 'error', 'critical'))
);

COMMENT ON TABLE observer.anomalies IS 'Anomaly events the observer detected. Bus emitters write here; Guard + Orchestrator subscribe.';

CREATE INDEX anomalies_tenant_ts_idx ON observer.anomalies(tenant_id, detected_at DESC);
CREATE INDEX anomalies_tenant_severity_idx ON observer.anomalies(tenant_id, severity);
CREATE INDEX anomalies_unresolved_idx ON observer.anomalies(tenant_id, detected_at DESC) WHERE resolved_at IS NULL;

ALTER TABLE observer.anomalies ENABLE ROW LEVEL SECURITY;

CREATE POLICY anomalies_tenant_select ON observer.anomalies
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

CREATE POLICY anomalies_tenant_modify ON observer.anomalies
    FOR ALL USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    )
    WITH CHECK (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );
