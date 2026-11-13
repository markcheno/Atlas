-- Slice 4.5: rescans that write only what changed, and a record of every
-- scan (phase4-spec §7.4; ADR 0022). Migration 0016 was reserved for OCR,
-- which was deferred.

-- Why a job was made: 'added' (a folder was added), 'launch' (a folder
-- never scanned, queued at open), 'user' (Rescan, Check all), 'automatic'
-- (the folder check). NULL for every job made before this migration: a
-- scan with no origin finishes the way it started, by Phase 0's rule.
ALTER TABLE jobs ADD COLUMN origin TEXT;

-- What a scan found, counted as it wrote: files new to the catalog, files
-- whose size, time or storage changed (or that came back), and files it
-- marked missing.
ALTER TABLE jobs ADD COLUMN new_files INTEGER NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN changed_files INTEGER NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN missing_files INTEGER NOT NULL DEFAULT 0;

-- Rows a scan found absent from their folder's listing. Marked missing
-- when the scan completes (never under a folder that failed to list, never
-- when the folder is unreachable), then deleted with the session's other
-- working rows. Kept on disk, so a scan resumed after a kill still knows.
CREATE TABLE scan_missing (
    session_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    file_id    TEXT NOT NULL REFERENCES file_instances(id) ON DELETE CASCADE,
    PRIMARY KEY (session_id, file_id)
) STRICT, WITHOUT ROWID;

-- What a scan couldn't read: folders it gave up on, with the reason's
-- code. Kept after the scan (at most 200 a scan, for a folder's last 10
-- scans) so the owner can see what was skipped and why.
CREATE TABLE scan_problems (
    id           INTEGER PRIMARY KEY,
    job_id       TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    rel_path     BLOB NOT NULL,
    display_path TEXT NOT NULL,
    code         TEXT NOT NULL
) STRICT;

CREATE INDEX scan_problems_by_job ON scan_problems (job_id);

-- A scan in progress keeps counting the way it began (by its marker on
-- the rows it wrote), so its live count starts where it was.
UPDATE jobs SET discovered_files =
        (SELECT COUNT(*) FROM file_instances f WHERE f.last_seen_session_id = jobs.id)
 WHERE kind = 'scan'
   AND state IN ('queued', 'running', 'paused_user', 'paused_policy', 'failed_retryable');
