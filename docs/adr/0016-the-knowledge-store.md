# ADR 0016: The knowledge store

- **Status:** Accepted (Phase 3, Slice 3.2)
- **Date:** 2026-11-11
- **Context documents:** [Phase 3 spec](../phase3-spec.md) §6, §8, §12;
  [ADR 0002](0002-data-root-creation-locking-and-compatibility.md) (the
  data root, refusing a newer catalog);
  [ADR 0011](0011-re-reading-without-losing-search.md) (what a re-read
  keeps)

## Context

Phase 3 lets the user keep things: research trails, bookmarks, saved
findings, interests and projects. Everything Atlas has stored so far can be
made again from the user's files. A catalog can be rebuilt by rescanning,
and an index by re-embedding. What the user creates can't be made again.
Three facts shape where it goes:

- **The catalog is large and busy.** It is gigabytes at 30,000 files, with
  jobs writing to it all the time. Copying it daily for a backup is too
  expensive, and damage to it would take everything in it along.
- **Catalog row ids don't last.**
  - `passages.id` and `chunks.id` are new after every re-read.
  - `file_instances.id` is new after every move or rename.
  - Only `documents.id` lasts, and a rebuilt catalog gives documents new
    ids. A content hash is the same in every catalog.
- **The store may be newer than the build reading it.** A newer Atlas may
  have written it, or a file may not be what Atlas expects. Today that
  makes the whole library unavailable (ADR 0002). For the user's own notes
  it must never lead to "repairing" the file.

## Decision

### A second database

- **`library/knowledge.sqlite`** holds what the user creates, with spec
  §6.1's tables (trails, trail steps and their documents, bookmarks,
  findings, collections and their items, dismissed suggestions, `meta`).
  The whole schema arrives in `knowledge/0001`, so Slices 3.3 to 3.6 add
  code, not migrations.
- **It refers to the catalog by value only.** There are no foreign keys
  across the two files. Each stays valid without the other.
- **Its own writer thread.** It is the catalog's code (`CatalogWriter`,
  opened as `atlas-knowledge-writer`) with WAL, `synchronous=NORMAL` and
  foreign keys on, plus a read pool of two connections.
- **Its own versions,** in `PRAGMA user_version`. The version is read
  before anything is written. Each migration runs in one transaction that
  also sets the version.

### A store that can't be used never blocks the library

When the library opens, the store ends in one of three states:

| State | When | What Atlas does |
| --- | --- | --- |
| `ready` | Opened and migrated | Normal use |
| `incompatible` | `user_version` above what this build supports | Leaves the file byte for byte; refuses every use with `knowledge_unavailable` |
| `damaged` | Not SQLite, someone else's SQLite file (version 0 with tables), or a migration failed | The same; Storage points to the snapshots |

The library opens in every case, and search and Ask work. This differs on
purpose from a newer catalog, which makes the library unavailable: without
the catalog nothing works, but without the store only the user's own
things are missing, and they must survive until a matching Atlas, or the
user, deals with them.

### Anchors

A bookmark, a finding's source and a trail step point at a place by an
**anchor**: the document's id and content hash, a title, the file type,
the page, location and passage ordinal as they were, and an excerpt of at
most 2,000 characters. `anchor::build` makes one from a file id and a
place, reading the catalog.

**`anchor::resolve`** runs each time an anchor is shown, against the
catalog as it is now, and never writes:

1. **The document:** by id if its content hash matches, else by content
   hash. Neither found means `unknown`.
2. **The file:** an available, local file of the document; else one only
   in the cloud. Neither means `no_file`. This covers a file moved away, a
   file whose bytes changed (now a different document) and a folder
   removed.
3. **The excerpt:** folded, with whitespace collapsed, it is looked for in
   this order:
   - the saved ordinal;
   - the document's other passages;
   - its first 160 characters, the same way.

   Found at the saved ordinal and page means `found`; found anywhere else
   means `found_elsewhere`, at the new place. Not found means `found` at
   the saved place, with `excerptFound: false`.

An anchor is never repaired: what was saved is the record of what the
user saw.

### Change tracking and snapshots

- **Triggers count changes.** `AFTER INSERT/UPDATE/DELETE` triggers on
  every user table bump `meta.changes`, so no later slice can forget to
  mark a change. Writes to `meta` itself don't count.
- **The rule:** a snapshot is taken when `meta.changes` differs from the
  count the last snapshot captured, and the newest snapshot is at least
  24 hours old, or there is none. It is checked when a library opens, and
  every hour by the store's own thread (a `Signal` wait, joined on drop).
  Time comes from a `Clock`: the system clock in the app, a settable
  `TestClock` in tests.
- **How a snapshot is taken:**
  1. `VACUUM INTO backups/knowledge/staging/…`, on the writer thread, so
     the copy and the count it records are of the same moment;
  2. `PRAGMA integrity_check` on the copy;
  3. an atomic rename to `backups/knowledge/knowledge-<UTC
     yyyymmddThhmmssZ>.sqlite`, then a sync of the folder;
  4. `last_snapshot_at` and `last_snapshot_change` recorded in `meta`.

  A copy that fails its check is deleted and reported. Opening a library
  empties the staging folder.
- **Kept:** the newest 14. The oldest goes only after a new one has passed
  its check, so a kill in between leaves 15 until the next snapshot.
- **Never touched** by Clean caches (only `cache/` and `tmp/`), by
  low-space handling or by any job. On a nearly full drive
  (`system::low_space_threshold`) no snapshot is taken, the reason shows
  in Storage, and nothing is deleted to make room.
- **Restoring is by hand in Phase 3:** quit Atlas, then copy a snapshot
  over `library/knowledge.sqlite` (and delete any
  `knowledge.sqlite-wal` and `-shm` beside it). See
  [architecture](../architecture.md#knowledge-store).

## Consequences

- "One SQLite database" no longer holds as a scope rule. The data root has
  two, closed in order: the store before the catalog (field order in
  `OpenRoot`).
- The catalog and the store can't be changed in one transaction. Later
  slices write the store after reading the catalog, and resolve anchors
  rather than trusting ids.
- An `incompatible` or `damaged` store blocks only the features that use
  it. Settings › Storage says what happened.
- A snapshot doubles the store's size on disk at most 14 times over. At
  the spec's 100,000 steps that is a few hundred MB, shown in Storage as
  "Snapshots (N)".
- The knowledge writer reuses the catalog's writer and reader code
  instead of the generic `sqlite_writer.rs` the plan proposed. The code
  had nothing catalog-specific beyond the thread's name, which is now a
  parameter (`CatalogWriter::open_named`).
