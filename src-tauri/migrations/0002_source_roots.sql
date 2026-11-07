-- Slice 0.3: folders the user authorized Atlas to read. Availability is a
-- live fact computed when listing, so it is not stored here.
CREATE TABLE source_roots (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspace(id),
    -- Native path, platform-tagged (infrastructure/native_path.rs).
    path_bytes   BLOB NOT NULL,
    -- Lossy UTF-8 rendering for display and filename search.
    display_path TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    last_scan_at TEXT,
    UNIQUE (workspace_id, path_bytes)
) STRICT;
