-- V026 — gateway identity bindings (REVIVAL P4, FR-GW-08 / SDS §9.4.2).
--
-- A messaging gateway (Telegram, Matrix, …) resolves an inbound platform handle
-- to a daimon IAM user through this table BEFORE any capability runs. A message
-- from an unmapped handle is refused — no anonymous actor ever dispatches (the
-- gateway equivalent of the console's C4 fix that closed the hardcoded
-- "operator" hole). Bindings are admin-enrolled through the Channels settings
-- tab, never self-service.
--
-- Single-org: no tenant column. The lookup key is UNIQUE(channel,
-- platform_handle). `user_id` -> public.users(id) ON DELETE CASCADE (removing a
-- user removes their channel bindings). `enrolled_by` records the admin who
-- created the binding, for the audit trail; ON DELETE SET NULL so deleting that
-- admin does not cascade-delete live bindings.

CREATE TABLE public.gateway_identities (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    channel         TEXT        NOT NULL,
    platform_handle TEXT        NOT NULL,
    user_id         UUID        NOT NULL REFERENCES public.users(id) ON DELETE CASCADE,
    enrolled_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    enrolled_by     UUID        REFERENCES public.users(id) ON DELETE SET NULL,
    UNIQUE (channel, platform_handle)
);

-- The reverse lookup (a user's bound handles) for the admin surface.
CREATE INDEX gateway_identities_user_idx ON public.gateway_identities (user_id);
