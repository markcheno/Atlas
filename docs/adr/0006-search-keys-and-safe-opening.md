# ADR 0006: Search keys and safe opening

- **Status:** Accepted
- **Date:** 2026-11-07
- **Context documents:** [Phase 0 spec](../phase0-spec.md) sections 5.5, 9 and 13;
  [ADR 0005](0005-inventory-durability.md)

## Context

Users must find files by name even when they type a different case or no
accents ("cafe" for "Café"), or when the filesystem returns decomposed
Unicode, as macOS often does. Results must appear while cataloging runs and
must stay fast on tens of thousands of files. Opening a file hands a path to
the operating system. That path must still be the cataloged file inside an
authorized folder, it must never be interpreted by a shell, and a
cloud-only file must not be downloaded without the user's consent.

## Decision

**Folded keys stored per row.** When it records a file, the inventory
worker stores `search_key` (the root-relative path) and `name_key` (the
file name). Both are folded the same way: decompose to NFD, drop combining
marks, lowercase, then recompose to NFC. Queries fold the user's text the
same way, split it on whitespace, and AND one
`search_key LIKE '%term%' ESCAPE '\'` per term, with `%`, `_` and `\`
escaped. Rows from Slice 0.5 are backfilled when a library opens, because
NFC folding can't be done in SQL. The data root and the source root's own
location are not part of the key, so searching a home-folder name doesn't
match everything.

**LIKE, not FTS5, for names.** A substring scan of 30,000 rows takes
about 30 ms (p95 33 ms in a debug build), well inside the 250 ms target,
and supports partial words such as "harmon". FTS5 arrives with document
text in Phase 1. Revisit name search then, or if libraries pass roughly
300,000 files and p95 approaches the target; an FTS5 trigram index over
`search_key` is the likely replacement.

**Separate read connections.** Two read-only connections, with
`query_only` set, serve UI queries. They never queue behind inventory
batches on the writer thread, and WAL lets them read while the writer
commits. A page reports `matchedSoFar` and `scanning`, so the UI says
"so far" instead of implying a total.

**Opening is by ID, validated in Rust.** The renderer sends only a file ID.
Rust looks up the root and relative path, then checks in order:

1. The source folder is reachable (a bounded probe).
2. The file exists. If only a legacy `.Name.icloud` placeholder exists,
   that is reported separately.
3. It is a regular file, not a symlink or anything else.
4. Its canonical path is still inside the canonical source folder.
5. It isn't dataless, re-read at that moment.

The native path then goes to `tauri-plugin-opener`, which uses OS APIs, or
`Command` with separate arguments, and never a shell string. The renderer
has no opener permission at all.

**Consent before download.** If a file is cloud-only at the moment of
opening, `open_original` refuses with `file_cloud_only` unless
`allowDownload` is true. The UI shows "Opening it will download it (size)"
with Download and open or Cancel. A legacy iCloud placeholder can't be
opened by its real name, so Atlas offers to show the placeholder in
Finder, where the user can download it. Reveal never downloads anything.

## Consequences

- Searches ignore case and accents in every script that composes accents.
  They don't do transliteration (for example ß to ss) or fuzzy matching.
- A file replaced by a symlink, or moved outside its folder through a
  symlinked parent, is refused until a rescan updates the catalog.
- Offset pagination can shift by a few rows while a scan inserts new
  files. Live refreshes run at most once per second.
