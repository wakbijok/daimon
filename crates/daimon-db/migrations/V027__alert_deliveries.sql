-- V027 — outbound alert delivery log (REVIVAL P6, FR-GW-13/15 / SDS §4.8.4).
--
-- Every attempt to route an alert (Observer-confirmed AnomalyDetected, Guard
-- AwaitingApproval) out to a messaging channel is recorded here, whether it
-- succeeded or failed. This is the fail-soft evidence trail (FR-GW-15): a
-- channel that is unreachable is LOGGED here and does NOT block the originating
-- loop, so operators/auditors can see "the anomaly fired but Telegram was down"
-- after the fact.
--
-- `app_config` needs no reconcile here: V017/V018/V019 already converged it to
-- the flat de-tenanted shape (key TEXT PK, value JSONB, is_secret, updated_by,
-- no RLS). P6 only adds this delivery log.
--
-- Denormalized on purpose: `channel` + `recipient` are stored as text (not an
-- FK to gateway_identities) so the log survives a later identity re-binding or
-- deletion — an audit record must not vanish because a handle was re-enrolled.
-- `signature` is the dedup key (anomaly signature or plan id) used by the
-- router's TTL admit() to collapse a flapping metric into one alert per window.

CREATE TABLE public.alert_deliveries (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    alert_class  TEXT        NOT NULL,               -- 'anomaly' | 'approval'
    severity     TEXT,                               -- optional, class-dependent
    signature    TEXT        NOT NULL,               -- dedup key (anomaly sig / plan id)
    channel      TEXT        NOT NULL,               -- 'telegram' | 'matrix'
    recipient    TEXT        NOT NULL,               -- platform handle / room id
    status       TEXT        NOT NULL,               -- 'delivered' | 'failed'
    detail       TEXT,                               -- error string on failure (no secrets)
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Dedup / throttle lookups are by (signature, created_at); the router asks
-- "did we already alert on this signature within the TTL window?".
CREATE INDEX alert_deliveries_signature_idx
    ON public.alert_deliveries (signature, created_at DESC);

-- Recent-first scan for the (P7) Incidents surface.
CREATE INDEX alert_deliveries_created_idx
    ON public.alert_deliveries (created_at DESC);

COMMENT ON TABLE public.alert_deliveries IS
    'Fail-soft outbound alert delivery log (P6, FR-GW-13/15). One row per delivery attempt; never blocks the originating loop.';
