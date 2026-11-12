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
| 13 | `0013_embed_jobs.sql` | the embedding job's counters on `jobs`: chunks and characters sent, time waiting on the service (Phase 2) |
| 14 | `0014_boilerplate.sql` | `documents.boilerplate_chars`: Project Gutenberg header and licence text the EPUB reader left out (Phase 3) |
| 15 | `0015_ocr_pages.sql` | `documents.text_pages`: a Needs OCR PDF's pages with usable text; PDFs marked Needs OCR before it are read once more for their counts (Phase 4) |

## Knowledge store migrations

`knowledge/` holds the migrations of `library/knowledge.sqlite`, the
user's trails and notes (Slice 3.2,
[ADR 0016](../../docs/adr/0016-the-knowledge-store.md)). They are
registered in `services/knowledge/migrations.rs`. They follow the rules
above, with two differences:

- The version is `PRAGMA user_version`, set in the same transaction as the
  migration. There is no `schema_migrations` table.
- A newer store, or a file that isn't one, is left untouched and the
  library opens anyway.

Every user table gets the three `meta.changes` triggers (`AFTER INSERT`,
`UPDATE`, `DELETE`), so snapshots know whether anything changed. A table
added later needs them too.

| Version | File | Adds |
| --- | --- | --- |
| 1 | `knowledge/0001_knowledge.sql` | `meta` (`changes`, `created_at`), `trails`, `trail_steps`, `step_documents`, `bookmarks`, `findings`, `collections`, `collection_items`, `dismissed_suggestions`, and the change triggers (Phase 3) |
