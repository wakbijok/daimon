-- V015 — Drop `observer.metrics`. Time-series tier (#4) moves to
-- VictoriaMetrics (Phase 8 lock per MASTERPLAN §3.5 amendment 2026-05-23
-- late). Metric streams now ingest via `VictoriaMetricsSink` against VM's
-- `/api/v1/import/prometheus` endpoint; Postgres retains only the
-- audit-adjacent `observer.anomalies` event log and the `observer.named_queries`
-- config table.
--
-- This migration is purely DROP — V014's anomalies + named_queries tables
-- are unchanged. Roll-forward only; rollback requires re-running V014's
-- metrics block by hand.

DROP INDEX IF EXISTS observer.metrics_labels_gin_idx;
DROP INDEX IF EXISTS observer.metrics_source_idx;
DROP INDEX IF EXISTS observer.metrics_tenant_ts_name_idx;

DROP POLICY IF EXISTS metrics_tenant_insert ON observer.metrics;
DROP POLICY IF EXISTS metrics_tenant_select ON observer.metrics;

DROP TABLE IF EXISTS observer.metrics;

COMMENT ON SCHEMA observer IS
    'Audit-adjacent event tier — anomalies + named_queries only. Metric streams live in VictoriaMetrics (Phase 8 lock).';
