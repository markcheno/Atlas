-- Slice 2.1: which saved AI service and model does each job (phase2-spec
-- §6.1, §7.1; ADR 0012).
--
-- A role is assigned only after a capability test of that model passed
-- (tested_at). dimensions: the embedding model's vector length, reported
-- by that test. Deleting a service clears its roles.
CREATE TABLE model_roles (
    role                TEXT PRIMARY KEY NOT NULL CHECK (role IN ('chat', 'embedding')),
    provider_profile_id TEXT NOT NULL REFERENCES provider_profiles (id) ON DELETE CASCADE,
    model_id            TEXT NOT NULL,
    dimensions          INTEGER,
    tested_at           TEXT NOT NULL,
    updated_at          TEXT NOT NULL
) STRICT;
