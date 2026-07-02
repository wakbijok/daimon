-- V019 — Drop tenant_id columns + recreate single-org indexes (FR-FND-08).
--
-- Dropping a column automatically drops the indexes that reference it and
-- the column's own outgoing FK to public.tenants, so we drop then recreate
-- the single-org equivalents (name/ref uniqueness that was previously
-- scoped by tenant now applies org-wide — which is the correct constraint
-- for one organization; this also closes the "names unique per tenant"
-- ambiguity by construction).
--
-- Scope note: public.users + public.role_grants (IAM) are handled in V022,
-- public.clusters is dropped whole in V021, public.tenants in V023, and
-- public.app_config was reconciled in V017 — none are touched here.

-- ---- vault.credentials ----
ALTER TABLE vault.credentials DROP COLUMN IF EXISTS tenant_id;
CREATE UNIQUE INDEX IF NOT EXISTS credentials_name_idx ON vault.credentials(name);

-- ---- inventory.targets ----
ALTER TABLE inventory.targets DROP COLUMN IF EXISTS tenant_id;
CREATE UNIQUE INDEX IF NOT EXISTS targets_ref_idx ON inventory.targets(target_ref);

-- ---- audit.events ----
ALTER TABLE audit.events DROP COLUMN IF EXISTS tenant_id;
CREATE INDEX IF NOT EXISTS events_ts_idx     ON audit.events(ts DESC);
CREATE INDEX IF NOT EXISTS events_actor_idx  ON audit.events(actor_id);
CREATE INDEX IF NOT EXISTS events_action_idx ON audit.events(action);
CREATE INDEX IF NOT EXISTS events_target_idx ON audit.events(target_ref) WHERE target_ref IS NOT NULL;
CREATE INDEX IF NOT EXISTS events_result_idx ON audit.events(result);

-- ---- audit.anchors ----
ALTER TABLE audit.anchors DROP COLUMN IF EXISTS tenant_id;
CREATE INDEX IF NOT EXISTS anchors_ts_idx ON audit.anchors(as_of_ts DESC);

-- ---- public.plans ----
ALTER TABLE public.plans DROP COLUMN IF EXISTS tenant_id;
CREATE INDEX IF NOT EXISTS plans_status_idx  ON public.plans(status);
CREATE INDEX IF NOT EXISTS plans_created_idx ON public.plans(created_at DESC);

-- ---- memory.documents ----
ALTER TABLE memory.documents DROP COLUMN IF EXISTS tenant_id;
CREATE UNIQUE INDEX IF NOT EXISTS documents_source_idx ON memory.documents(source_id);
CREATE INDEX        IF NOT EXISTS documents_kind_idx   ON memory.documents(source_kind);

-- ---- memory.document_chunks ----
ALTER TABLE memory.document_chunks DROP COLUMN IF EXISTS tenant_id;

-- ---- observer.anomalies ----
ALTER TABLE observer.anomalies DROP COLUMN IF EXISTS tenant_id;
CREATE INDEX IF NOT EXISTS anomalies_ts_idx         ON observer.anomalies(detected_at DESC);
CREATE INDEX IF NOT EXISTS anomalies_severity_idx   ON observer.anomalies(severity);
CREATE INDEX IF NOT EXISTS anomalies_unresolved_idx ON observer.anomalies(detected_at DESC) WHERE resolved_at IS NULL;

-- ---- public.approvals ----
ALTER TABLE public.approvals DROP COLUMN IF EXISTS tenant_id;
CREATE INDEX IF NOT EXISTS approvals_status_idx  ON public.approvals(status);
CREATE INDEX IF NOT EXISTS approvals_created_idx ON public.approvals(created_at DESC);
