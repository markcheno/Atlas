-- Slice 0.2: the local workspace. One row per data root in Phase 0;
-- the table exists so future multi-workspace and hosted designs keep IDs.
CREATE TABLE workspace (
    id         TEXT PRIMARY KEY NOT NULL,
    name       TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;
