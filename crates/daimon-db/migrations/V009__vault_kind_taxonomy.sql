-- V009 — align vault.credentials.kind CHECK with daimon-vault's CredentialKind.
--
-- V003 was written with a placeholder taxonomy (ssh, token, password, tls,
-- api_key). The actual Rust enum variants are: SshKey, SshPassword, ApiToken,
-- Generic. Drop the placeholder constraint and replace with the real one.
--
-- Per crates/daimon-vault/src/credential.rs `enum CredentialKind`.

ALTER TABLE vault.credentials
    DROP CONSTRAINT credentials_kind_check;

ALTER TABLE vault.credentials
    ADD CONSTRAINT credentials_kind_check
    CHECK (kind IN ('SshKey', 'SshPassword', 'ApiToken', 'Generic'));
