-- Slice 0.6: accent- and case-insensitive filename/path search.
-- Keys are folded in Rust (NFD, drop combining marks, lowercase, NFC);
-- rows from earlier versions are backfilled when the library opens.
ALTER TABLE file_instances ADD COLUMN search_key TEXT;
ALTER TABLE file_instances ADD COLUMN name_key TEXT;

CREATE INDEX file_instances_name_sort ON file_instances (availability, name_key);
CREATE INDEX file_instances_root_name_sort ON file_instances (source_root_id, availability, name_key);
CREATE INDEX file_instances_date_sort ON file_instances (availability, mtime_secs);
CREATE INDEX file_instances_unkeyed ON file_instances (search_key) WHERE search_key IS NULL;
