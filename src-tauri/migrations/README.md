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
