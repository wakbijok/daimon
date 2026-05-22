-- V001 — create the four schemas that organise daimon's relational tier.
--
-- public    — tenants, users, roles, role_grants, plan_history, clusters
-- vault     — credentials (per-row encryption, KMS-backed master in Phase 2c D4)
-- inventory — managed targets (target://<name> → host+port+xport+credential_ref)
-- audit     — append-only event log with hash chain (Phase 2c D5)
--
-- Per MASTERPLAN.md §3.3 and plans/2026-05-23-phase-2c-compliance-posture-plan.md D2.

CREATE SCHEMA IF NOT EXISTS vault;
CREATE SCHEMA IF NOT EXISTS inventory;
CREATE SCHEMA IF NOT EXISTS audit;

-- public schema already exists by default — no CREATE needed.

COMMENT ON SCHEMA vault IS 'Credential vault — per-row ciphertext + KMS-backed master DEK.';
COMMENT ON SCHEMA inventory IS 'Managed target registry — target://<name> → endpoint + credential_ref.';
COMMENT ON SCHEMA audit IS 'Append-only event log. Triggers block UPDATE/DELETE. Hash-chained per tenant.';
