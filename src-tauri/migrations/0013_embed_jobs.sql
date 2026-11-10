-- Slice 2.3: the embedding job's counters (phase2-spec §7.2; ADR 0014).
--
-- Jobs of kind 'embed' record how many chunks they sent to the embedding
-- service, how many characters, and how long they waited for it, so the
-- status line and the benchmarks can report the service's throughput.
-- (The new kind itself needs no change: `jobs.kind` has no constraint. The
-- version bump makes an older Atlas refuse a library whose jobs it can't
-- read, untouched.)
ALTER TABLE jobs ADD COLUMN embedded_chunks INTEGER NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN sent_chars INTEGER NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN service_ms INTEGER NOT NULL DEFAULT 0;
