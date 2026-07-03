-- V028 — alert delivery correlation (REVIVAL P7-2, FR-GW-14).
--
-- Approve-over-chat reply correlation needs to map an inbound REPLY back to the
-- approval it decides. When the alert router delivers an approval alert, the
-- gateway now returns the provider's id of the SENT message (Telegram
-- message_id, Matrix event_id); we record it here alongside the delivery row.
-- On an inbound reply, `(channel, provider_message_id)` → the delivery row whose
-- `signature` is the approval id, so the reply is correlated server-side and the
-- decision applied — surviving a console restart (persisted, not in-memory).
--
-- Nullable + additive: existing delivery rows (and anomaly alerts, which are not
-- replied to) simply carry NULL. No backfill.

ALTER TABLE public.alert_deliveries
    ADD COLUMN provider_message_id TEXT;

-- The reverse lookup on reply: "which approval does this (channel, message) reply
-- to?". Partial index — only rows that carry a provider id are correlatable.
CREATE INDEX alert_deliveries_reply_lookup_idx
    ON public.alert_deliveries (channel, provider_message_id)
    WHERE provider_message_id IS NOT NULL;

COMMENT ON COLUMN public.alert_deliveries.provider_message_id IS
    'Provider id of the sent alert message (Telegram message_id / Matrix event_id), for approve-over-chat reply correlation (P7-2).';
