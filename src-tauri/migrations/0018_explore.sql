-- Slice 4.6.1: Explore, a map of the library by meaning (ADR 0024). Every
-- table here is derived from the embeddings and can be rebuilt; the owner's
-- names for areas are the only choice it holds, and survive a rebuild where
-- their area does.

-- One vector per book per model: the mean of a sample of its chunks'
-- vectors, as Related documents computes it (f16, unit length). Written as
-- each book is embedded, and filled in for books embedded before 4.6.1.
CREATE TABLE book_vectors (
    model_id     TEXT NOT NULL REFERENCES embedding_models (id) ON DELETE CASCADE,
    document_id  TEXT NOT NULL REFERENCES documents (id) ON DELETE CASCADE,
    vector       BLOB NOT NULL,
    chunk_count  INTEGER NOT NULL,
    -- The document_embeddings.seq the vector was made from.
    updated_seq  INTEGER NOT NULL,
    PRIMARY KEY (model_id, document_id)
) STRICT;

-- The map of one model's books: when it was drawn, from what.
CREATE TABLE explore_maps (
    model_id     TEXT PRIMARY KEY NOT NULL REFERENCES embedding_models (id) ON DELETE CASCADE,
    generation   INTEGER NOT NULL,
    books        INTEGER NOT NULL,
    -- The last document_embeddings.seq the map has seen, and the book
    -- pairs' stamp: what changed since is what makes it stale.
    through_seq  INTEGER NOT NULL,
    pairs_stamp  TEXT NOT NULL,
    computed_at  TEXT NOT NULL,
    ms           INTEGER NOT NULL
) STRICT;

-- Areas, and their sub-areas (parent_id set). `number` and `hue` are kept
-- across rebuilds for an area that survives; `name` is the owner's.
CREATE TABLE explore_areas (
    id         INTEGER PRIMARY KEY,
    model_id   TEXT NOT NULL REFERENCES embedding_models (id) ON DELETE CASCADE,
    parent_id  INTEGER REFERENCES explore_areas (id) ON DELETE CASCADE,
    number     INTEGER NOT NULL,
    hue        INTEGER NOT NULL CHECK (hue BETWEEN 0 AND 7),
    -- JSON arrays: the area's words, and its central books' document ids.
    words      TEXT NOT NULL,
    name       TEXT,
    central    TEXT NOT NULL,
    size       INTEGER NOT NULL,
    cohesion   REAL NOT NULL
) STRICT;

CREATE INDEX explore_areas_by_model ON explore_areas (model_id, parent_id);

-- Where each book sits on the map, and in which area and sub-area.
CREATE TABLE explore_points (
    model_id     TEXT NOT NULL REFERENCES embedding_models (id) ON DELETE CASCADE,
    document_id  TEXT NOT NULL REFERENCES documents (id) ON DELETE CASCADE,
    x            REAL NOT NULL,
    y            REAL NOT NULL,
    area_id      INTEGER NOT NULL REFERENCES explore_areas (id) ON DELETE CASCADE,
    sub_area_id  INTEGER REFERENCES explore_areas (id) ON DELETE SET NULL,
    PRIMARY KEY (model_id, document_id)
) STRICT;

CREATE INDEX explore_points_by_area ON explore_points (area_id);
CREATE INDEX explore_points_by_sub_area ON explore_points (sub_area_id)
    WHERE sub_area_id IS NOT NULL;
