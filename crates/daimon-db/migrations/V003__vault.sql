-- V003 — vault.credentials.
--
-- Per-row XChaCha20-Poly1305 ciphertext payload, wrapped under a master DEK.
-- Phase 2c D4 swaps the master from a local file to a KMS-wrapped DEK; the
-- `encryption_version` column lets us identify which DEK generation sealed
-- a row, enabling online re-encryption during rotation.
--
-- Per MASTERPLAN.md §3.3 + §4.2 + §4.5 and
-- plans/2026-05-23-phase-2c-compliance-posture-plan.md D2 + D4.

CREATE TABLE vault.credentials (
    id                  UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           UUID         NOT NULL REFERENCES public.tenants(id) ON DELETE RESTRICT,
    name                TEXT         NOT NULL,
    kind                TEXT         NOT NULL,
    payload_sealed      BYTEA        NOT NULL,
    encryption_version  INTEGER      NOT NULL DEFAULT 1,
    created_at          TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ  NOT NULL DEFAULT now(),
    CHECK (kind IN ('ssh', 'token', 'password', 'tls', 'api_key'))
);

COMMENT ON TABLE vault.credentials IS 'Per-row sealed credential payload. Plaintext never persisted. Master DEK held in process memory, wrapped by KMS at rest (D4).';
COMMENT ON COLUMN vault.credentials.payload_sealed IS 'XChaCha20-Poly1305 ciphertext: nonce || ciphertext || tag. Bincode-serialized Credential enum prior to seal.';
COMMENT ON COLUMN vault.credentials.encryption_version IS 'DEK generation that sealed this row. Incremented on rotation (daimon vault rotate-dek).';

-- Name unique within tenant only — different tenants may share credential names.
CREATE UNIQUE INDEX credentials_tenant_name_idx
    ON vault.credentials(tenant_id, name);
CREATE INDEX credentials_tenant_idx ON vault.credentials(tenant_id);
CREATE INDEX credentials_kind_idx ON vault.credentials(kind);

CREATE TRIGGER credentials_touch_updated_at
    BEFORE UPDATE ON vault.credentials
    FOR EACH ROW EXECUTE FUNCTION public.touch_updated_at();
