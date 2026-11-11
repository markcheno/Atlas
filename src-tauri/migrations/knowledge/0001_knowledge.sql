-- The knowledge store, version 1 (Phase 3, Slice 3.2; phase3-spec §6.1,
-- ADR 0016): everything the user creates. It refers to the catalog by
-- value (document ids and content hashes), never by foreign key, so each
-- file stays valid without the other. Its version is PRAGMA user_version.

CREATE TABLE meta (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL) STRICT;
-- changes: bumped by the triggers below on every write to a user table, so
-- a snapshot knows whether anything changed since the last one.
INSERT INTO meta (key, value) VALUES ('changes', '0');

CREATE TABLE trails (
    id           TEXT PRIMARY KEY NOT NULL,
    title        TEXT,
    pinned       INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    search_text  TEXT NOT NULL DEFAULT '',
    step_count   INTEGER NOT NULL DEFAULT 0,
    started_at   TEXT NOT NULL,
    last_step_at TEXT NOT NULL
) STRICT;
CREATE INDEX trails_by_activity ON trails (last_step_at DESC);

CREATE TABLE trail_steps (
    id        TEXT PRIMARY KEY NOT NULL,
    trail_id  TEXT NOT NULL REFERENCES trails (id) ON DELETE CASCADE,
    seq       INTEGER NOT NULL,
    kind      TEXT NOT NULL CHECK (kind IN ('search', 'ask', 'open', 'note')),
    at        TEXT NOT NULL,
    payload   TEXT NOT NULL,
    note      TEXT,
    UNIQUE (trail_id, seq)
) STRICT;

CREATE TABLE step_documents (
    step_id      TEXT NOT NULL REFERENCES trail_steps (id) ON DELETE CASCADE,
    document_id  TEXT NOT NULL,
    content_hash BLOB NOT NULL,
    PRIMARY KEY (step_id, document_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX step_documents_by_document ON step_documents (document_id);

CREATE TABLE bookmarks (
    id           TEXT PRIMARY KEY NOT NULL,
    anchor       TEXT NOT NULL,
    document_id  TEXT NOT NULL,
    content_hash BLOB NOT NULL,
    note         TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
) STRICT;
CREATE INDEX bookmarks_by_document ON bookmarks (document_id);

CREATE TABLE findings (
    id           TEXT PRIMARY KEY NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN ('answer', 'passage')),
    question     TEXT,
    text         TEXT NOT NULL,
    sources      TEXT NOT NULL,
    note         TEXT,
    from_step_id TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
) STRICT;

CREATE TABLE collections (
    id          TEXT PRIMARY KEY NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('interest', 'project')),
    name        TEXT NOT NULL,
    description TEXT,
    goal        TEXT,
    status      TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'finished', 'archived')),
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
) STRICT;

CREATE TABLE collection_items (
    collection_id TEXT NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    item_kind     TEXT NOT NULL CHECK (item_kind IN ('trail', 'bookmark', 'finding', 'document')),
    item_id       TEXT NOT NULL,
    content_hash  BLOB,
    added_at      TEXT NOT NULL,
    PRIMARY KEY (collection_id, item_kind, item_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE dismissed_suggestions (
    collection_id TEXT NOT NULL,
    document_id   TEXT NOT NULL,
    content_hash  BLOB NOT NULL,
    at            TEXT NOT NULL,
    PRIMARY KEY (collection_id, document_id)
) STRICT, WITHOUT ROWID;

-- Every write to a user table counts as a change (not writes to meta).
CREATE TRIGGER trails_changed_insert AFTER INSERT ON trails BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER trails_changed_update AFTER UPDATE ON trails BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER trails_changed_delete AFTER DELETE ON trails BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER trail_steps_changed_insert AFTER INSERT ON trail_steps BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER trail_steps_changed_update AFTER UPDATE ON trail_steps BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER trail_steps_changed_delete AFTER DELETE ON trail_steps BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER step_documents_changed_insert AFTER INSERT ON step_documents BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER step_documents_changed_update AFTER UPDATE ON step_documents BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER step_documents_changed_delete AFTER DELETE ON step_documents BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER bookmarks_changed_insert AFTER INSERT ON bookmarks BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER bookmarks_changed_update AFTER UPDATE ON bookmarks BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER bookmarks_changed_delete AFTER DELETE ON bookmarks BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER findings_changed_insert AFTER INSERT ON findings BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER findings_changed_update AFTER UPDATE ON findings BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER findings_changed_delete AFTER DELETE ON findings BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER collections_changed_insert AFTER INSERT ON collections BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER collections_changed_update AFTER UPDATE ON collections BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER collections_changed_delete AFTER DELETE ON collections BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER collection_items_changed_insert AFTER INSERT ON collection_items BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER collection_items_changed_update AFTER UPDATE ON collection_items BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER collection_items_changed_delete AFTER DELETE ON collection_items BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER dismissed_suggestions_changed_insert AFTER INSERT ON dismissed_suggestions BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER dismissed_suggestions_changed_update AFTER UPDATE ON dismissed_suggestions BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
CREATE TRIGGER dismissed_suggestions_changed_delete AFTER DELETE ON dismissed_suggestions BEGIN
    UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'changes';
END;
