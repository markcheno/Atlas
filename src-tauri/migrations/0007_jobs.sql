-- Phase 1, Slice 1.1: scan sessions become general jobs (phase1-spec §6.1,
-- ADR 0009). Every existing row is a scan; its state, counters and frontier
-- (scan_directories, whose foreign key SQLite rewrites to the new name) are
-- kept as they are.
--
-- `kind` has no CHECK constraint on purpose: adding a kind would then need a
-- table rebuild, and rebuilding a table that scan_directories references
-- cascades under the runner's foreign keys. The Rust `JobKind` enum refuses
-- unknown values instead.

ALTER TABLE scan_sessions RENAME TO jobs;

ALTER TABLE jobs ADD COLUMN kind TEXT NOT NULL DEFAULT 'scan';

DROP INDEX scan_sessions_by_root;
DROP INDEX scan_sessions_by_state;

CREATE INDEX jobs_by_root ON jobs (source_root_id, kind);
CREATE INDEX jobs_by_kind_state ON jobs (kind, state, priority, created_at);
