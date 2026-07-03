-- V029 — durable user-scoped chat history (REVIVAL P7-3, FR-UI-18/19, SDS §11.5).
--
-- Before P7, chat history lived only in the browser (localStorage) + the Redis
-- working-memory hot tier (TTL-bounded). Neither survives browser-storage loss,
-- a different browser, or the retention window. These two tables make the
-- transcript DURABLE and OWNER-SCOPED:
--
--   chat_sessions — one row per conversation, owned by an IAM user.
--   chat_turns    — the ordered messages of a session.
--
-- Ownership is enforced in the #[server] fn body (owner_id == authenticated
-- subject, admin/auditor read-override), NOT a row policy — RLS was dropped in
-- V018 (single-org). The durable transcript is a CONVENIENCE record; the
-- hash-chained audit trail remains the independent accountability source
-- (FR-UI-21), so this is deliberately NOT on the audit chain.
--
-- session id is TEXT (matches the existing session ids: a browser uuid, or a
-- gateway `gw:<channel>:<thread>` id) — the same key the Redis hot tier uses.

CREATE TABLE public.chat_sessions (
    id          TEXT        PRIMARY KEY,
    owner_id    UUID        NOT NULL REFERENCES public.users(id) ON DELETE CASCADE,
    title       TEXT        NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE public.chat_turns (
    id          BIGSERIAL   PRIMARY KEY,
    session_id  TEXT        NOT NULL REFERENCES public.chat_sessions(id) ON DELETE CASCADE,
    role        TEXT        NOT NULL,          -- 'user' | 'assistant' | 'tool'
    content     TEXT        NOT NULL,
    tool_use_id TEXT,                          -- correlates a tool call/result, when applicable
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Ordered replay of a session's turns.
CREATE INDEX chat_turns_session_idx ON public.chat_turns (session_id, id);

-- A user's session list, most-recent first.
CREATE INDEX chat_sessions_owner_idx ON public.chat_sessions (owner_id, updated_at DESC);

-- Retention prune (P7-6) scans by age.
CREATE INDEX chat_sessions_updated_idx ON public.chat_sessions (updated_at);

COMMENT ON TABLE public.chat_sessions IS
    'Durable, owner-scoped chat sessions (P7, FR-UI-18). Convenience transcript; audit chain is the accountability source.';
