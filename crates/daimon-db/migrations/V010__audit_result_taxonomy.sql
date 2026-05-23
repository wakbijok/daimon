-- V010 — align audit.events.result CHECK with daimon-audit's AuditResult.
--
-- V005 placeholder was (success, failure, denied, partial). Actual Rust
-- enum is (Success, Error, Denied) serialized as (success, error, denied).
-- Drop placeholder + replace.
--
-- Per crates/daimon-audit/src/event.rs `enum AuditResult`.

ALTER TABLE audit.events
    DROP CONSTRAINT events_result_check;

ALTER TABLE audit.events
    ADD CONSTRAINT events_result_check
    CHECK (result IN ('success', 'error', 'denied'));
