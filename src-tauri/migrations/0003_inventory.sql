-- Slice 0.5: durable inventory. A scan session is the job record
-- (phase0-spec §6.1, §7.3); its directory frontier makes scans resumable.

CREATE TABLE scan_sessions (
    id                  TEXT PRIMARY KEY NOT NULL,
    source_root_id      TEXT NOT NULL REFERENCES source_roots(id) ON DELETE CASCADE,
    state               TEXT NOT NULL CHECK (state IN ('queued', 'running', 'paused_user',
                            'paused_policy', 'failed_retryable', 'failed_terminal',
                            'cancelled', 'completed')),
    pause_reason        TEXT,
    throttle_reason     TEXT,
    priority            INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT NOT NULL,
    started_at          TEXT,
    finished_at         TEXT,
    updated_at          TEXT NOT NULL,
    scanned_directories INTEGER NOT NULL DEFAULT 0,
    discovered_files    INTEGER NOT NULL DEFAULT 0,
    skipped_symlinks    INTEGER NOT NULL DEFAULT 0,
    skipped_entries     INTEGER NOT NULL DEFAULT 0,
    error_count         INTEGER NOT NULL DEFAULT 0,
    retry_count         INTEGER NOT NULL DEFAULT 0,
    elapsed_ms          INTEGER NOT NULL DEFAULT 0,
    last_error          TEXT
) STRICT;

CREATE INDEX scan_sessions_by_root ON scan_sessions (source_root_id, created_at);
CREATE INDEX scan_sessions_by_state ON scan_sessions (state, priority, created_at);

-- Directories still to list for a session. Deleted when the session ends.
CREATE TABLE scan_directories (
    id         INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES scan_sessions(id) ON DELETE CASCADE,
    -- Native path relative to the source root (empty for the root itself).
    rel_path   BLOB NOT NULL,
    state      TEXT NOT NULL CHECK (state IN ('pending', 'done', 'failed')),
    attempts   INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    UNIQUE (session_id, rel_path)
) STRICT;

CREATE INDEX scan_directories_next ON scan_directories (session_id, state, attempts, id);

CREATE TABLE file_instances (
    id                   TEXT PRIMARY KEY NOT NULL,
    source_root_id       TEXT NOT NULL REFERENCES source_roots(id) ON DELETE CASCADE,
    -- Native path relative to the source root.
    rel_path             BLOB NOT NULL,
    display_path         TEXT NOT NULL,
    display_name         TEXT NOT NULL,
    extension            TEXT NOT NULL,
    file_type            TEXT NOT NULL CHECK (file_type IN ('pdf', 'epub')),
    -- NULL when unknown (for example an unreadable cloud placeholder).
    size_bytes           INTEGER,
    -- Whole seconds: FAT/exFAT and some copy tools drop sub-second parts.
    mtime_secs           INTEGER,
    modified_at          TEXT,
    availability         TEXT NOT NULL CHECK (availability IN ('available', 'missing')),
    -- 'cloud_only': contents offloaded to iCloud, OneDrive, or similar.
    -- Atlas lists such files without downloading them.
    storage              TEXT NOT NULL DEFAULT 'local' CHECK (storage IN ('local', 'cloud_only')),
    last_seen_session_id TEXT,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL,
    UNIQUE (source_root_id, rel_path)
) STRICT;

CREATE INDEX file_instances_by_name ON file_instances (source_root_id, display_name);
CREATE INDEX file_instances_by_session ON file_instances (last_seen_session_id);
CREATE INDEX file_instances_by_availability ON file_instances (availability, source_root_id);
