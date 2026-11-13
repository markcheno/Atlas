-- Slice 4.5.1: one book, two formats (ADR 0023). A PDF and an EPUB in one
-- folder with the same name apart from the extension, whose texts match, are
-- one book: one is indexed for meaning and shown, the other is kept out of
-- what Atlas shows and counts. Nothing already in the catalog changes.

-- One PDF and one EPUB, both available, in the same folder of one source
-- root, with the same name apart from the extension (folded as names are).
-- Rebuilt from the names; it holds no decision.
CREATE TABLE format_pairs (
    pdf_file_id    TEXT PRIMARY KEY NOT NULL REFERENCES file_instances (id) ON DELETE CASCADE,
    epub_file_id   TEXT NOT NULL UNIQUE REFERENCES file_instances (id) ON DELETE CASCADE,
    -- The file the Library folds into its partner's row while the two
    -- documents are one book; NULL otherwise.
    folded_file_id TEXT,
    created_at     TEXT NOT NULL
) STRICT;

CREATE INDEX format_pairs_folded ON format_pairs (folded_file_id)
    WHERE folded_file_id IS NOT NULL;

-- Whether two documents are one book: the text check's verdict, the name
-- alone (one side has no text), or the owner's choice, which is kept.
CREATE TABLE book_pairs (
    pdf_document_id    TEXT NOT NULL REFERENCES documents (id) ON DELETE CASCADE,
    epub_document_id   TEXT NOT NULL REFERENCES documents (id) ON DELETE CASCADE,
    verdict            TEXT NOT NULL CHECK (verdict IN ('same', 'different')),
    basis              TEXT NOT NULL CHECK (basis IN ('text', 'name', 'owner')),
    -- The text check's measures: the share of the smaller text's sample
    -- found in the larger, and the ratio of their lengths in words.
    similarity         REAL,
    length_ratio       REAL,
    -- Which format is indexed for meaning and shown, while 'same'. Kept once
    -- chosen: a copy already indexed stays the one used.
    index_format       TEXT CHECK (index_format IN ('pdf', 'epub')),
    check_version      INTEGER NOT NULL,
    -- Each document's updated_at when checked: a re-read is checked again.
    pdf_checked_at     TEXT NOT NULL,
    epub_checked_at    TEXT NOT NULL,
    -- While the book is active (the same, and side by side in a folder): the
    -- document indexed and shown, and the one kept out.
    index_document_id  TEXT,
    hidden_document_id TEXT,
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL,
    PRIMARY KEY (pdf_document_id, epub_document_id),
    CHECK ((index_document_id IS NULL) = (hidden_document_id IS NULL))
) STRICT;

CREATE UNIQUE INDEX book_pairs_hidden ON book_pairs (hidden_document_id)
    WHERE hidden_document_id IS NOT NULL;
CREATE UNIQUE INDEX book_pairs_index ON book_pairs (index_document_id)
    WHERE index_document_id IS NOT NULL;
CREATE INDEX book_pairs_by_epub ON book_pairs (epub_document_id);

-- Which version of the pairing pass has run over each folder; 0 = never, so
-- an existing library pairs its books once, the first time it opens.
ALTER TABLE source_roots ADD COLUMN formats_version INTEGER NOT NULL DEFAULT 0;
