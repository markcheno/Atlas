-- Slice 0.8: the indexing status (get_inventory_status, sent with every
-- progress event) counts available, missing and cloud-only files in total
-- and per folder. Without an index holding `storage`, both counts read the
-- whole file_instances table: about 20 ms per call at 30,000 files
-- (docs/benchmarks.md). This covering index makes them narrow index scans.
CREATE INDEX file_instances_status_counts
    ON file_instances (source_root_id, availability, storage);
