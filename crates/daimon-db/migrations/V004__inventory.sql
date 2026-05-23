-- V004 — inventory.targets.
--
-- Managed targets that daimon agents act on via the broker.
-- A target_ref looks like `target://<slug>` and is unique within a tenant.
--
-- Per MASTERPLAN.md §3.3 and plans/2026-05-23-phase-2c-compliance-posture-plan.md D2.

CREATE TABLE inventory.targets (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID         NOT NULL REFERENCES public.tenants(id) ON DELETE RESTRICT,
    target_ref      TEXT         NOT NULL,
    kind            TEXT         NOT NULL,
    transport       TEXT         NOT NULL,
    host            TEXT         NOT NULL,
    port            INTEGER      NOT NULL,
    credential_ref  TEXT         NOT NULL,
    labels          JSONB        NOT NULL DEFAULT '{}'::jsonb,
    capabilities    JSONB        NOT NULL DEFAULT '[]'::jsonb,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT now(),
    CHECK (port > 0 AND port <= 65535),
    CHECK (target_ref LIKE 'target://%'),
    CHECK (credential_ref LIKE 'vault://%')
);

COMMENT ON TABLE inventory.targets IS 'Managed endpoints. credential_ref points into vault.credentials; broker resolves at action time.';
COMMENT ON COLUMN inventory.targets.labels IS 'Free-form key/value labels for selection (env, region, tier, etc.).';
COMMENT ON COLUMN inventory.targets.capabilities IS 'JSON array of capability slugs this target supports (informational; broker authoritative).';

CREATE UNIQUE INDEX targets_tenant_ref_idx ON inventory.targets(tenant_id, target_ref);
CREATE INDEX targets_tenant_idx ON inventory.targets(tenant_id);
CREATE INDEX targets_kind_idx ON inventory.targets(kind);
CREATE INDEX targets_transport_idx ON inventory.targets(transport);
CREATE INDEX targets_labels_gin_idx ON inventory.targets USING gin(labels);

CREATE TRIGGER targets_touch_updated_at
    BEFORE UPDATE ON inventory.targets
    FOR EACH ROW EXECUTE FUNCTION public.touch_updated_at();
