-- V021 — Drop the PVE cluster registry (REVIVAL P0, FR-FND-13).
--
-- PVE is OUT: daimon-pve is deleted and its capability generalizes into
-- the platform-agnostic target-connector framework (P2), which reaches
-- targets through the broker/inventory, not a bespoke clusters table.
-- CASCADE removes the table's trigger + indexes; its RLS policies were
-- already dropped in V018. The shared touch_updated_at() function is used
-- by other tables and is NOT dropped.

DROP TABLE IF EXISTS public.clusters CASCADE;
