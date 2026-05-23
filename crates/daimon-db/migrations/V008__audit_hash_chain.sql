-- V008 — audit.events hash chain trigger.
--
-- Per-tenant chain. Canonical payload for sha256 input:
--   ts_iso8601_utc || '|' || actor_id || '|' || action || '|' || target_ref
--   || '|' || credential_ref || '|' || op_summary || '|' || result
--   || '|' || latency_ms_decimal || '|' || metadata_jsonb_text
--
-- NULL values canonicalize to the empty string. JSONB::text in Postgres
-- emits keys in a stable order, so it is safe to use directly. Genesis
-- prev_hash for a tenant's first row is 32 zero bytes.
--
-- row_hash = sha256( canonical_utf8_bytes || prev_hash_bytes )
--
-- BEFORE INSERT trigger populates prev_hash and row_hash so they are
-- committed atomically with the row itself. Append-only triggers from
-- V005 still block UPDATE/DELETE, so the chain cannot be edited after
-- the fact.
--
-- Per MASTERPLAN.md §4.3 and plans/2026-05-23-phase-2c-compliance-posture-plan.md D5.

CREATE EXTENSION IF NOT EXISTS pgcrypto;  -- for digest('sha256')

CREATE OR REPLACE FUNCTION audit.compute_row_hash() RETURNS trigger AS $$
DECLARE
    prev BYTEA;
    canonical TEXT;
BEGIN
    SELECT row_hash INTO prev
    FROM audit.events
    WHERE tenant_id = NEW.tenant_id
    ORDER BY ts DESC, id DESC
    LIMIT 1;

    IF prev IS NULL THEN
        prev := decode('0000000000000000000000000000000000000000000000000000000000000000', 'hex');
    END IF;

    NEW.prev_hash := prev;

    canonical := COALESCE(to_char(NEW.ts AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), '')
              || '|' || COALESCE(NEW.actor_id, '')
              || '|' || COALESCE(NEW.action, '')
              || '|' || COALESCE(NEW.target_ref, '')
              || '|' || COALESCE(NEW.credential_ref, '')
              || '|' || COALESCE(NEW.op_summary, '')
              || '|' || COALESCE(NEW.result, '')
              || '|' || COALESCE(NEW.latency_ms::TEXT, '')
              || '|' || COALESCE(NEW.metadata::TEXT, '{}');

    NEW.row_hash := digest(convert_to(canonical, 'UTF8') || prev, 'sha256');

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER events_compute_hash
    BEFORE INSERT ON audit.events
    FOR EACH ROW EXECUTE FUNCTION audit.compute_row_hash();

COMMENT ON FUNCTION audit.compute_row_hash IS 'Per-tenant hash chain. Computes prev_hash + row_hash on BEFORE INSERT. Deterministic UTF-8 canonical payload.';

-- ---- anchor manifests --------------------------------------------------------
--
-- Cluster-wide manifest store. Each entry captures a tenant chain head at
-- a point in time. Prod deploys mirror entries to S3 Object Lock or a
-- WORM volume (daimon-anchor binary handles the external write).

CREATE TABLE audit.anchors (
    id                  UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           UUID         NOT NULL REFERENCES public.tenants(id) ON DELETE RESTRICT,
    as_of_ts            TIMESTAMPTZ  NOT NULL,
    row_hash            BYTEA        NOT NULL,
    row_count           BIGINT       NOT NULL,
    daimon_instance_id  UUID         NOT NULL,
    signature           BYTEA,
    created_at          TIMESTAMPTZ  NOT NULL DEFAULT now()
);

COMMENT ON TABLE audit.anchors IS 'Cluster-wide tamper-evidence anchors. One row per (tenant, as_of_ts). Mirrored to S3 Object Lock / WORM by daimon-anchor.';
COMMENT ON COLUMN audit.anchors.signature IS 'Optional KMS signature over the anchor payload (D4 prod). NULL in dev / file-anchor mode.';

CREATE INDEX anchors_tenant_ts_idx ON audit.anchors(tenant_id, as_of_ts DESC);
CREATE INDEX anchors_instance_idx ON audit.anchors(daimon_instance_id);

-- Same append-only enforcement as audit.events.
CREATE TRIGGER anchors_no_update
    BEFORE UPDATE ON audit.anchors
    FOR EACH ROW EXECUTE FUNCTION audit.block_update_delete();

CREATE TRIGGER anchors_no_delete
    BEFORE DELETE ON audit.anchors
    FOR EACH ROW EXECUTE FUNCTION audit.block_update_delete();
