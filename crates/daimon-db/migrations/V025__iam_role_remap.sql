-- V025 — IAM role-slug remap (REVIVAL P1).
--
-- V022 already collapsed the tenant SCOPE dimension (dropped role_grants.scope,
-- dropped users.tenant_id, added the plain UNIQUE(user_id, role_id), and seeded
-- the single-org `admin` + `approver` roles). What remains for P1 is the
-- role-SLUG remap: the legacy multi-tenant roles (cluster_admin, tenant_admin,
-- viewer) fold into the single-org catalogue, and any grants pointing at them
-- are repointed onto their single-org successor BEFORE the legacy rows are
-- deleted — role_grants.role_id has an ON DELETE RESTRICT FK to roles.id, so
-- deleting a role that still has grants would abort the whole migration.
--
-- Final catalogue after this migration: {admin, operator, approver, read-only,
-- auditor}. `operator` and `auditor` are untouched.
--
-- Idempotent: seeds use ON CONFLICT DO NOTHING; the repoint UPDATEs carry a
-- NOT-EXISTS collision guard so a user who already holds the target role does
-- not violate UNIQUE(user_id, role_id); a leftover-grant DELETE mops up any
-- grant the guard skipped; the role DELETE is unconditional but a no-op on
-- replay once the slugs are gone.

-- ---- (a) cluster_admin / tenant_admin -> admin ------------------------------
-- `admin` was seeded in V022; ensure it exists (idempotent belt).
INSERT INTO public.roles (slug, name, description, is_system) VALUES
    ('admin', 'Administrator',
     'Full administration — users, credentials, targets, connectors, settings, audit.', true)
ON CONFLICT (slug) DO NOTHING;

-- Repoint legacy admin grants onto `admin`, skipping any that would collide
-- with a grant the user already holds (avoids UNIQUE(user_id, role_id) breach).
UPDATE public.role_grants rg
   SET role_id = (SELECT id FROM public.roles WHERE slug = 'admin')
 WHERE rg.role_id IN (SELECT id FROM public.roles WHERE slug IN ('cluster_admin', 'tenant_admin'))
   AND NOT EXISTS (
       SELECT 1 FROM public.role_grants x
        WHERE x.user_id = rg.user_id
          AND x.role_id = (SELECT id FROM public.roles WHERE slug = 'admin')
   );

-- Any legacy admin grant the collision guard skipped is now redundant (the user
-- already holds `admin`) — drop it so the role DELETE below is unblocked.
DELETE FROM public.role_grants
 WHERE role_id IN (SELECT id FROM public.roles WHERE slug IN ('cluster_admin', 'tenant_admin'));

-- ---- (b) viewer -> read-only ------------------------------------------------
-- Seed the single-org read-only role (V022 did NOT seed this one).
INSERT INTO public.roles (slug, name, description, is_system) VALUES
    ('read-only', 'Read-only',
     'Read-only access to dashboards, chat, and audit.', true)
ON CONFLICT (slug) DO NOTHING;

-- Repoint viewer grants onto `read-only`, same collision guard.
UPDATE public.role_grants rg
   SET role_id = (SELECT id FROM public.roles WHERE slug = 'read-only')
 WHERE rg.role_id IN (SELECT id FROM public.roles WHERE slug = 'viewer')
   AND NOT EXISTS (
       SELECT 1 FROM public.role_grants x
        WHERE x.user_id = rg.user_id
          AND x.role_id = (SELECT id FROM public.roles WHERE slug = 'read-only')
   );

-- Mop up any viewer grant the guard skipped.
DELETE FROM public.role_grants
 WHERE role_id IN (SELECT id FROM public.roles WHERE slug = 'viewer');

-- ---- (c) delete the retired role rows (AFTER grants repointed) --------------
-- FK role_grants.role_id -> roles.id is ON DELETE RESTRICT; this only succeeds
-- because (a) and (b) left zero grants pointing at these slugs.
DELETE FROM public.roles WHERE slug IN ('cluster_admin', 'tenant_admin', 'viewer');
