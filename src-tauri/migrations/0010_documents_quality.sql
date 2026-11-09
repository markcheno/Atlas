-- Slice 1.6: metadata quality and stay-searchable re-reading.
--
-- content_title: a title read from a PDF's first page (its largest line),
-- used when the document's own metadata has no usable title.
-- refresh: 1 while the document is due to be read again by a newer reader
-- (see READER_VERSION_* in engine/mod.rs). Its files stay linked, so the
-- old text stays searchable until the new text replaces it in one
-- transaction.
ALTER TABLE documents ADD COLUMN content_title TEXT;
ALTER TABLE documents ADD COLUMN refresh INTEGER NOT NULL DEFAULT 0;
CREATE INDEX documents_refresh ON documents (refresh) WHERE refresh = 1;

-- Duplicate groups count available copies per document.
DROP INDEX file_instances_by_document;
CREATE INDEX file_instances_by_document ON file_instances (document_id, availability);
