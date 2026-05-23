-- V005 — audit.events.
--
-- Append-only event log. Hash-chain columns (prev_hash, row_hash) are
-- populated by triggers landed in D5; this migration creates the columns
-- and the schema, plus DB-level UPDATE/DELETE blockers (defence in depth
-- on top of the AuditSink API).
--
-- Hash-chain semantics (D5):
--   prev_hash = previous row's row_hash for the SAME tenant (NULL for first row)
--   row_hash  = sha256(canonical_payload || prev_hash)
-- Per-tenant chains avoid global lock contention.
--
-- Per MASTERPLAN.md §4.3 and plans/2026-05-23-phase-2c-compliance-posture-plan.md D2 + D5.

CREATE TABLE audit.events (
    id               UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        UUID         NOT NULL REFERENCES public.tenants(id) ON DELETE RESTRICT,
    ts               TIMESTAMPTZ  NOT NULL DEFAULT now(),
    actor_id         TEXT         NOT NULL,
    action           TEXT         NOT NULL,
    target_ref       TEXT,
    credential_ref   TEXT,
    op_summary       TEXT,
    result           TEXT         NOT NULL,
    latency_ms       INTEGER,
    metadata         JSONB        NOT NULL DEFAULT '{}'::jsonb,
    prev_hash        BYTEA,
    row_hash         BYTEA,
    CHECK (result IN ('success', 'failure', 'denied', 'partial'))
);

COMMENT ON TABLE audit.events IS 'Append-only structured event log. Hash-chained per tenant. UPDATE/DELETE blocked at DB level.';
COMMENT ON COLUMN audit.events.prev_hash IS 'Previous row_hash for this tenant. NULL for the first row per tenant.';
COMMENT ON COLUMN audit.events.row_hash IS 'sha256(canonical_payload || prev_hash). Populated by INSERT trigger in D5.';
COMMENT ON COLUMN audit.events.metadata IS 'Per-action JSON metadata. Schema varies by action; consumers should treat unknown keys as additive.';

CREATE INDEX events_tenant_ts_idx ON audit.events(tenant_id, ts DESC);
CREATE INDEX events_tenant_actor_idx ON audit.events(tenant_id, actor_id);
CREATE INDEX events_tenant_action_idx ON audit.events(tenant_id, action);
CREATE INDEX events_tenant_target_idx ON audit.events(tenant_id, target_ref) WHERE target_ref IS NOT NULL;
CREATE INDEX events_tenant_result_idx ON audit.events(tenant_id, result);

-- Append-only enforcement: UPDATE and DELETE blocked at DB level.
CREATE OR REPLACE FUNCTION audit.block_update_delete() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'audit.events is append-only — % blocked', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER events_no_update
    BEFORE UPDATE ON audit.events
    FOR EACH ROW EXECUTE FUNCTION audit.block_update_delete();

CREATE TRIGGER events_no_delete
    BEFORE DELETE ON audit.events
    FOR EACH ROW EXECUTE FUNCTION audit.block_update_delete();
