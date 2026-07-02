-- V023 — Drop public.tenants (REVIVAL P0, FR-FND-08). MUST be last of the
-- tenancy rip-out: every FK into public.tenants was removed by V019
-- (leaf tenant_id columns), V021 (clusters), and V022 (users), so the
-- table now has no dependents. CASCADE is defensive; nothing should remain
-- to cascade. The V002 'default' tenant seed disappears with the table.

DROP TABLE IF EXISTS public.tenants CASCADE;
