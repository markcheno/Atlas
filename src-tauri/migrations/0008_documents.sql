-- Phase 1, Slice 1.3: documents, their extracted text, and per-file
-- extraction state (phase1-spec §6.1, ADR 0009).
--
-- A document is a file's content, identified by the BLAKE3-256 of its bytes;
-- several file instances (copies, moves) can share one. It carries the text,
-- the extraction outcome and the metadata. The file instance carries only
-- what blocks it before hashing (cloud-only, unreadable, hash failed).
--
-- Passages (one per PDF page, or per EPUB chapter piece) are created here so
-- Slice 1.3's text is kept; Slice 1.4's migration adds only the FTS5 index
-- over them. Order matters: tables are created before anything references
-- them, because the migration runner keeps foreign keys on.

CREATE TABLE documents (
    id                TEXT PRIMARY KEY NOT NULL,
    content_hash      BLOB NOT NULL UNIQUE,
    state             TEXT NOT NULL CHECK (state IN ('pending', 'running', 'done', 'needs_ocr',
                          'encrypted', 'malformed', 'unsupported', 'too_large',
                          'worker_crashed', 'failed')),
    -- A fixed reason code (never a path), e.g. 'not_a_pdf', 'zip_slip'.
    reason            TEXT,
    extractor_version INTEGER,
    page_count        INTEGER,
    char_count        INTEGER,
    title             TEXT,
    -- JSON array of strings.
    authors           TEXT,
    language          TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
) STRICT;

CREATE INDEX documents_state ON documents (state);

CREATE TABLE passages (
    id          INTEGER PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES documents (id) ON DELETE CASCADE,
    ordinal     INTEGER NOT NULL,
    -- PDF page number (1-based); NULL for EPUB pieces.
    page_no     INTEGER,
    -- What the UI shows: "Page 12", "Chapter 3, part 2".
    location    TEXT,
    text        TEXT NOT NULL,
    UNIQUE (document_id, ordinal)
) STRICT;

ALTER TABLE file_instances ADD COLUMN extract_state TEXT NOT NULL DEFAULT 'pending'
    CHECK (extract_state IN ('pending', 'linked', 'skipped_cloud_only', 'unreadable',
                             'hash_failed'));
-- Size and whole-second mtime the hash was computed from.
ALTER TABLE file_instances ADD COLUMN hashed_size INTEGER;
ALTER TABLE file_instances ADD COLUMN hashed_mtime_secs INTEGER;
ALTER TABLE file_instances ADD COLUMN document_id TEXT REFERENCES documents (id);
ALTER TABLE file_instances ADD COLUMN extract_error TEXT;
-- Worker crashes while reading this file; two make it 'worker_crashed'.
ALTER TABLE file_instances ADD COLUMN extract_attempts INTEGER NOT NULL DEFAULT 0;

CREATE INDEX file_instances_extract_queue
    ON file_instances (source_root_id, availability, extract_state);
CREATE INDEX file_instances_by_document ON file_instances (document_id);
