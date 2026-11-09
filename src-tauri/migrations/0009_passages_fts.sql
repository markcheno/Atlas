-- Phase 1, Slice 1.4: full-text search over the stored passages
-- (phase1-spec §6.1, §7.4, ADR 0010).
--
-- An external-content FTS5 index: the text stays in `passages` (0008); the
-- index stores only what search needs. `unicode61 remove_diacritics 2`
-- folds case and accents, as the Phase 0 name search does. The three
-- triggers are the ones the SQLite FTS5 documentation gives for external
-- content tables; the index is never maintained by hand.
--
-- 'rebuild' indexes the passages Slice 1.3 already stored, once, so nothing
-- is extracted again.

CREATE VIRTUAL TABLE passages_fts USING fts5 (
    text,
    content = 'passages',
    content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2',
    detail = 'full'
);

CREATE TRIGGER passages_fts_insert AFTER INSERT ON passages BEGIN
    INSERT INTO passages_fts (rowid, text) VALUES (new.id, new.text);
END;

CREATE TRIGGER passages_fts_delete AFTER DELETE ON passages BEGIN
    INSERT INTO passages_fts (passages_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;

CREATE TRIGGER passages_fts_update AFTER UPDATE ON passages BEGIN
    INSERT INTO passages_fts (passages_fts, rowid, text) VALUES ('delete', old.id, old.text);
    INSERT INTO passages_fts (rowid, text) VALUES (new.id, new.text);
END;

INSERT INTO passages_fts (passages_fts) VALUES ('rebuild');
