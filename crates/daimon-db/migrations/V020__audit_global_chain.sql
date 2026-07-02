-- V020 — Collapse the audit hash chain to a single global chain (FR-FND-08,
-- and fixes the concurrent-insert chain-fork bug M-audit-chain-lock).
--
-- V008 chained per-tenant (`WHERE tenant_id = NEW.tenant_id`). tenant_id is
-- gone (V019), so the chain is now one global sequence. Two changes vs V008:
--   1. Remove the tenant predicate from the prev-hash lookup.
--   2. Take a transaction-scoped advisory lock before reading the chain head,
--      so two concurrent inserts can't both read the same prev_hash and fork
--      the chain (which would raise a false tamper alarm on verify).
-- The canonical payload is byte-for-byte identical to V008 (it never included
-- tenant_id), so previously-written rows remain verifiable in-format.
--
-- Legal to CREATE OR REPLACE: function bodies are not checksummed by refinery,
-- only the migration file is. Runs after V019 so the new body does not
-- reference the dropped column.

CREATE OR REPLACE FUNCTION audit.compute_row_hash() RETURNS trigger AS $$
DECLARE
    prev BYTEA;
    canonical TEXT;
BEGIN
    -- Serialize chain-head reads across concurrent inserts (one global chain).
    PERFORM pg_advisory_xact_lock(4923011);

    SELECT row_hash INTO prev
    FROM audit.events
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

COMMENT ON FUNCTION audit.compute_row_hash IS
    'Single global hash chain (single-org). Advisory-locked chain-head read prevents concurrent-insert forks. Computes prev_hash + row_hash on BEFORE INSERT.';
