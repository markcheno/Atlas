# Catalog migrations

Ordered, append-only SQL files named `NNNN_description.sql`. Each is
embedded in the binary (`services/catalog/migrations.rs`), applied in its
own transaction, and recorded in `schema_migrations(version, name,
applied_at)`, which the runner creates itself.

Rules:

- Never edit or renumber a migration that has been committed; add a new one.
- Register every new file in `MIGRATIONS` in the same change.
- The highest registered version is the catalog schema version this build
  supports. A catalog with a higher version is refused, never written.
- The runner's connection has foreign keys **on** (`PRAGMA foreign_keys`
  can't be changed inside a transaction), so:
  - never add a `REFERENCES` clause to a table that doesn't exist yet: every
    later insert into the child table fails with "no such table";
  - never drop and rebuild a table that other tables reference: the drop
    cascades to their rows. This is why `jobs.kind` (0007) has no CHECK
    constraint; the Rust `JobKind` enum refuses unknown kinds instead.
- `ALTER TABLE … RENAME TO` is safe: the bundled SQLite rewrites foreign
  keys in other tables to the new name (0007 renamed `scan_sessions` to
  `jobs`; its test asserts the rewrite).

| Version | File | Adds |
| --- | --- | --- |
| 1 | `0001_initial.sql` | `workspace` |
| 2 | `0002_source_roots.sql` | `source_roots` |
| 3 | `0003_inventory.sql` | scan sessions, the directory frontier, `file_instances` |
| 4 | `0004_search_keys.sql` | folded search and sort keys |
| 5 | `0005_provider_profiles.sql` | `provider_profiles` |
| 6 | `0006_status_counts_index.sql` | covering index for the status counts |
| 7 | `0007_jobs.sql` | `scan_sessions` becomes `jobs` with a `kind` (Phase 1) |
| 8 | `0008_documents.sql` | `documents`, `passages`, and the `file_instances` extraction columns (Phase 1) |
| 9 | `0009_passages_fts.sql` | the FTS5 index over `passages`, its triggers, and a one-time rebuild (Phase 1) |
| 10 | `0010_documents_quality.sql` | `documents.content_title` and `refresh` (stay-searchable re-reading), and the `(document_id, availability)` index for duplicates (Phase 1) |
| 11 | `0011_model_roles.sql` | `model_roles`: the service and model for chat and for embeddings (Phase 2) |
| 12 | `0012_embeddings.sql` | `embedding_models`, `chunks`, `embeddings` (f16 BLOBs by text hash), `document_embeddings`, `vector_tombstones` and its trigger (Phase 2) |
