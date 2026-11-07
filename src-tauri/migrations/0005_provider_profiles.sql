-- Slice 0.7: AI service connections. API keys are never stored here: they
-- live in the OS keychain (account "provider:<id>") or, if the keychain is
-- unavailable, in memory for one session only.
CREATE TABLE provider_profiles (
    id                         TEXT PRIMARY KEY NOT NULL,
    kind                       TEXT NOT NULL CHECK (kind IN ('ollama', 'openai_compatible')),
    label                      TEXT NOT NULL,
    base_url                   TEXT NOT NULL,
    -- Where the key was put when saved; NULL means no key.
    credential_storage         TEXT CHECK (credential_storage IN ('keychain', 'session_only')),
    insecure_http_acknowledged INTEGER NOT NULL DEFAULT 0,
    created_at                 TEXT NOT NULL,
    updated_at                 TEXT NOT NULL
) STRICT;
