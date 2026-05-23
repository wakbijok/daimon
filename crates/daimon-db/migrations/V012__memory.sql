-- V012 — memory schema for Phase 3 canonical content tier.
--
-- Qdrant holds embeddings + sparse vectors. Postgres holds the source-of-
-- truth payload (document metadata + chunk text). One row per ingested
-- document; one row per chunk. Chunk PK is the same u64 hash as the Qdrant
-- point_id so retrievals can JOIN by id.
--
-- Per MASTERPLAN.md §3.2 + §3.3 and the Phase 3 plan D5 (canonical payload
-- tier) folded out of Phase 2c.

CREATE SCHEMA IF NOT EXISTS memory;

COMMENT ON SCHEMA memory IS 'Canonical content tier for the RAG pipeline. Qdrant holds embeddings; this schema holds the source-of-truth text + metadata.';

-- ---- memory.documents -------------------------------------------------------

CREATE TABLE memory.documents (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID         NOT NULL REFERENCES public.tenants(id) ON DELETE RESTRICT,
    source_id       TEXT         NOT NULL,
    source_kind     TEXT         NOT NULL,
    content_hash    BYTEA        NOT NULL,
    metadata        JSONB        NOT NULL DEFAULT '{}'::jsonb,
    ingested_at     TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT now()
);

COMMENT ON TABLE memory.documents IS 'Ingested document headers. One per (tenant, source_id) — re-ingest replaces.';
COMMENT ON COLUMN memory.documents.content_hash IS 'sha256 of the original content. Lets re-ingest skip the embed cost when unchanged.';

CREATE UNIQUE INDEX documents_tenant_source_idx
    ON memory.documents(tenant_id, source_id);
CREATE INDEX documents_tenant_kind_idx ON memory.documents(tenant_id, source_kind);

CREATE TRIGGER documents_touch_updated_at
    BEFORE UPDATE ON memory.documents
    FOR EACH ROW EXECUTE FUNCTION public.touch_updated_at();

-- ---- memory.document_chunks -------------------------------------------------

CREATE TABLE memory.document_chunks (
    -- BIGINT not UUID — the chunk id IS the Qdrant point id (u64 hash of
    -- source_id + chunk_index). Avoids a per-chunk join lookup table.
    -- Stored as BIGINT (signed), so cast on the way in/out: the upper bit
    -- of the u64 hash becomes the sign. Order doesn't matter — equality is
    -- what we need.
    id              BIGINT       PRIMARY KEY,
    document_id     UUID         NOT NULL REFERENCES memory.documents(id) ON DELETE CASCADE,
    tenant_id       UUID         NOT NULL REFERENCES public.tenants(id) ON DELETE RESTRICT,
    chunk_index     INTEGER      NOT NULL,
    content         TEXT         NOT NULL,
    word_start      INTEGER      NOT NULL,
    word_end        INTEGER      NOT NULL,
    token_estimate  INTEGER      NOT NULL DEFAULT 0,
    ingested_at     TIMESTAMPTZ  NOT NULL DEFAULT now()
);

COMMENT ON TABLE memory.document_chunks IS 'Per-chunk canonical text. id matches the Qdrant point id (u64-as-i64). Retrieval JOINs by id.';
COMMENT ON COLUMN memory.document_chunks.token_estimate IS 'Rough token count for the context-budget packer (D6c). Computed at ingest.';

CREATE INDEX chunks_document_idx ON memory.document_chunks(document_id);
CREATE INDEX chunks_tenant_idx ON memory.document_chunks(tenant_id);
CREATE UNIQUE INDEX chunks_document_index_idx
    ON memory.document_chunks(document_id, chunk_index);

-- ---- row-level security -----------------------------------------------------

ALTER TABLE memory.documents ENABLE ROW LEVEL SECURITY;

CREATE POLICY documents_tenant_select ON memory.documents
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

CREATE POLICY documents_tenant_modify ON memory.documents
    FOR ALL USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    )
    WITH CHECK (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

ALTER TABLE memory.document_chunks ENABLE ROW LEVEL SECURITY;

CREATE POLICY chunks_tenant_select ON memory.document_chunks
    FOR SELECT USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );

CREATE POLICY chunks_tenant_modify ON memory.document_chunks
    FOR ALL USING (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    )
    WITH CHECK (
        public.current_role_is_cluster_admin()
        OR tenant_id = public.current_tenant_id()
    );
