-- Slice 2.2: chunks, embeddings and the vector index's bookkeeping
-- (phase2-spec §6.1; ADR 0013).
--
-- Embeddings are durable data in the catalog; the usearch index under
-- indexes/vectors/ is derived from them and can always be rebuilt.

-- One row per embedding model (a model name at a dimension count and a
-- chunker version). Its id is the index file's stem.
CREATE TABLE embedding_models (
    id                       TEXT PRIMARY KEY NOT NULL,
    provider_profile_id      TEXT REFERENCES provider_profiles (id) ON DELETE SET NULL,
    model_name               TEXT NOT NULL,
    dims                     INTEGER NOT NULL CHECK (dims > 0),
    quantization             TEXT NOT NULL CHECK (quantization IN ('f16')),
    chunker_version          INTEGER NOT NULL,
    query_prefix             TEXT,
    status                   TEXT NOT NULL
        CHECK (status IN ('building', 'ready', 'retired', 'failed')),
    -- The last saved index file: <id>-<index_generation>.usearch, its BLAKE3,
    -- the last document_embeddings.seq it holds, how many vectors, and the
    -- last tombstone (vector_tombstones.seq) it has applied.
    index_generation         INTEGER NOT NULL DEFAULT 0,
    index_blake3             BLOB,
    indexed_through_seq      INTEGER NOT NULL DEFAULT 0,
    vector_count             INTEGER NOT NULL DEFAULT 0,
    tombstones_through       INTEGER NOT NULL DEFAULT 0,
    created_at               TEXT NOT NULL,
    updated_at               TEXT NOT NULL
) STRICT;

-- Sub-passages of at most 1,200 characters. char_start and char_end are
-- character (Unicode scalar) offsets into passages.text. The id is the
-- usearch key: AUTOINCREMENT so a deleted id is never reused (a tombstone
-- must never remove a newer chunk). Chunks are shared by every model.
CREATE TABLE chunks (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL REFERENCES documents (id) ON DELETE CASCADE,
    passage_id  INTEGER NOT NULL REFERENCES passages (id) ON DELETE CASCADE,
    ordinal     INTEGER NOT NULL,
    char_start  INTEGER NOT NULL,
    char_end    INTEGER NOT NULL CHECK (char_end > char_start),
    text_hash   BLOB NOT NULL,
    UNIQUE (document_id, ordinal)
) STRICT;

CREATE INDEX chunks_by_passage ON chunks (passage_id);

-- Content-addressed: identical chunk text is embedded once per model.
-- vector: dims × f16, little-endian, unit length. A rowid table: a
-- WITHOUT ROWID table keeps whole 2 KB rows in its interior pages, which
-- nearly doubled the bytes per vector in the Slice 2.2 benchmark.
CREATE TABLE embeddings (
    id        INTEGER PRIMARY KEY,
    model_id  TEXT NOT NULL REFERENCES embedding_models (id) ON DELETE CASCADE,
    text_hash BLOB NOT NULL,
    vector    BLOB NOT NULL,
    UNIQUE (model_id, text_hash)
) STRICT;

-- Per document, per model. No row means the document is waiting. seq
-- grows with every write for the model: the index holds the chunks of the
-- model's done documents, and a saved file records the last seq it holds.
CREATE TABLE document_embeddings (
    document_id TEXT NOT NULL REFERENCES documents (id) ON DELETE CASCADE,
    model_id    TEXT NOT NULL REFERENCES embedding_models (id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL,
    status      TEXT NOT NULL CHECK (status IN ('done', 'empty', 'failed')),
    reason      TEXT,
    chunk_count INTEGER NOT NULL DEFAULT 0,
    attempts    INTEGER NOT NULL DEFAULT 0,
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (document_id, model_id)
) STRICT;

CREATE INDEX document_embeddings_by_model ON document_embeddings (model_id, status);
CREATE UNIQUE INDEX document_embeddings_by_seq ON document_embeddings (model_id, seq);

-- Deleted chunk ids, in order. Chunks are shared by every model, so a
-- tombstone stays until every live model's saved index has applied it
-- (embedding_models.tombstones_through).
CREATE TABLE vector_tombstones (
    seq      INTEGER PRIMARY KEY AUTOINCREMENT,
    chunk_id INTEGER NOT NULL
) STRICT;

CREATE TRIGGER chunks_tombstone AFTER DELETE ON chunks BEGIN
    INSERT INTO vector_tombstones (chunk_id) VALUES (old.id);
END;
