# ADR 0019: Export and import, and where Atlas may write outside its data root

- **Status:** Accepted (Phase 3, Slice 3.8). Amends
  [ADR 0001](0001-local-ipc-no-server-single-data-root.md)'s list of
  exceptions.
- **Date:** 2026-11-11
- **Context documents:** [Phase 3 spec](../phase3-spec.md) §5.8, §7.7, §8,
  §12 (P3-22 to P3-24), §13; [ADR 0016](0016-the-knowledge-store.md) (the
  knowledge store); [ADR 0017](0017-trails-and-privacy.md) (trails and
  privacy)

## Context

Everything the user makes in Atlas lives in one file,
`library/knowledge.sqlite`. That includes trails, bookmarks, findings,
interests and projects. Phase 3 promises that all of it can leave and come
back. It can leave as Markdown notes for a notes app, or as one JSON file
that is a complete copy. A JSON file can be imported again, into the same
library or another.

Five facts shaped the design:

- **ADR 0001 keeps all of Atlas's data under one data root.** An export is
  the first time Atlas writes a file the user will keep somewhere else, and
  the first write into a folder Atlas didn't make.
- **The folders a user exports into already hold their own notes.** An
  Obsidian vault is the obvious place, and nothing in it may be lost.
- **Library folders are read-only to Atlas** (ADR 0006), and an export must
  not land in one.
- **An imported file is untrusted.** It may be truncated, edited by hand,
  from a newer Atlas, or crafted to name paths.
- **Trails had no `updated_at`.** A rename, a pin or a step's note left no
  time behind, so there was no way to tell which of two copies is newer.

## Decision

### Where Atlas may write

- **The folder comes from a native dialog** shown by Rust:
  `blocking_pick_folder`, outside the workspace lock, starting at the last
  export's folder or at the data root's `exports`. The renderer never sends
  a path.
- **Atlas writes only inside an `Atlas` folder it makes there.** It is
  refused (`export_folder_unusable`) if an `Atlas` there is a file or a
  link. The same goes for its subfolders `Trails`, `Findings`, `Bookmarks`,
  `Interests` and `Projects`.
- **Refused folders:**
  - a library folder, a folder inside one, or one whose `Atlas` would be or
    contain one (`export_inside_library`);
  - a folder inside the data root other than inside its `exports`, or one
    whose `Atlas` would be or contain the data root
    (`export_inside_data_root`).

  Folders are compared by filesystem identity (`sources::relation`), so
  links, case and Unicode forms don't get round the rules.
- **A file is replaced only if its front matter carries the same
  `atlas_id`.** Anything else is left alone, and the note goes beside it as
  `<name> 2.md`, `3`, and so on. That includes the user's own file, Atlas's
  note for another item, a link or a folder. Atlas never deletes a file
  there, so a renamed trail leaves its old note in place.
- **Names are made safe** for macOS, Windows, Linux and Obsidian links:
  - `<>:"/\|?*`, `#`, `[`, `]`, `^` and control characters become spaces;
  - a leading dot and trailing dots and spaces go;
  - reserved Windows names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`9`,
    `LPT1`–`9`, with or without an extension) get `_`;
  - names are cut to 100 characters;
  - within one export, names are compared ignoring case.
- **Every file is staged under a temporary name in its folder, then
  renamed into place.** A rename replaces a directory entry and never writes
  through a link. Notes are copies that can be written again, so each one
  is not flushed to disk by itself; the folders are flushed once at the end.
  Flushing each note took 8 s for 1,000 notes on macOS, against 0.2 s now.

ADR 0001's list of exceptions gains: *export folders the user chooses in a
native dialog, and only the `Atlas` folder Atlas makes inside one*.

### What is written

- **Markdown:** `Trails/<date> <title>.md`, `Findings/<date> <first
  words>.md`, `Bookmarks/<document title>.md` (one note per document),
  `Interests/<name>.md` and `Projects/<name>.md`.
  - **Front matter:** `atlas_id`, `atlas_kind`, `created`, `updated` and
    `exported`, as quoted YAML strings.
  - **Links:** `[[Trails/…]]` (Obsidian) or relative, percent-encoded
    Markdown links, as chosen.
  - **A source** is its title, page or place, and its file's path as text,
    or "no file now".
  - **Recorded text:** excerpts and notes are quoted line by line, and
    headings are kept to one line, so recorded text can't break a note.
  - **Dates** in names are the UTC date.
- **JSON:** one file, `Atlas/atlas-knowledge-<date>.json`
  (`format: "atlas-knowledge"`, `version: 1`).
  - **Rows:** every row in scope, typed as Atlas keeps them, with anchors
    and hex hashes. What can be worked out again is left out: step counts,
    filter text, and which documents a step touched.
  - **No paths at all.**
  - **Never replaced:** a second file the same day goes beside the first
    with a number.
- **Scope:** a trail, a finding, a collection, or everything. A collection
  brings its trails, findings and bookmarked documents with it. A single
  bookmark has no Export of its own; it goes with its document's note.

### Import

1. **The file** comes only from a native file dialog shown by Rust. It must
   be:
   - at most 1 GiB (256 MiB until Slice 3.9), checked from its size and
     again while reading;
   - whole JSON, or `import_unreadable`;
   - a known format and version, or `import_unknown_version`.
2. **Every row is checked like user input** (`import_invalid`, with a fixed
   reason):
   - UUIDs, and times exactly as Atlas writes them (UTC, to the second), so
     times compare as text;
   - enum values and payload version 1;
   - Atlas's own text limits;
   - sequence numbers unique within a trail;
   - unknown fields refused;
   - at most 1,000,000 rows;
   - every collection link naming an item in the file or already here.

   A bad file is refused whole, and nothing is written.
3. **A dry run** (`preview_import`) compares the file with the store and
   says what is new, newer or already here. It also counts the new,
   unpinned trails that expiry would remove. The parsed file waits under a
   token for 10 minutes, or until the library closes, and applies once.
4. **Applying** (`apply_import`) takes a snapshot first and refuses if one
   can't be taken (`import_no_snapshot`), so an import can always be undone.
   Then one transaction runs the same code as the dry run.
5. **Merging:** new items are added, the newer copy wins by `updated_at`,
   and nothing is ever deleted.
   - **A trail** keeps steps the file lacks and adds the file's new ones,
     renumbered by time. Each step, and the trail's title and pin, comes
     from whichever copy is newer.
   - **A bookmark** is matched by id, or by its place, as saving does.
   - **Links and "Not for this"** are only added. A link removed since the
     export comes back.

### Knowledge store version 2

Migration `0002_updated.sql` adds `updated_at` to `trails` and
`trail_steps`. Existing rows take their last step's time or the step's own
time. It changes on:

- a rename, a pin or a step's note;
- a step added, merged or deleted;
- an import.

An older Atlas refuses a version-2 store as `incompatible` and leaves it
untouched (ADR 0016).

### The exporter interface

`Exporter { id(); export(&ExportSet, &mut ExportDest) }`, product spec §10,
has two built-in implementations, `MarkdownExporter` and `JsonExporter`.
It returns Atlas's own error type. Nothing loads exporters at run time.

## Consequences

- **What the user has made can leave Atlas** in a form they can read and
  keep, and nothing of theirs in the destination is ever overwritten.
- **Atlas writes outside its data root,** but only into a folder the user
  chose in that moment, and only inside the `Atlas` folder there.
- **Imports are reversible** through the snapshot taken first, and dry
  runs show the outcome.
- **Exported Markdown isn't tracked.** Editing a note doesn't come back
  into Atlas, and Atlas doesn't remove a note whose item was deleted.
- **The import cap is 1 GiB** (Slice 3.9, the product owner's choice). The
  first cap, 256 MiB, held only about 100,000 steps of the generator's
  realistic sizes: that export was 247.5 MiB. The cap now holds about
  400,000 steps.
  - **Memory:** import's peak was about 2.4 times the file (575 MiB above
    the start for a 244 MiB file, on every OS). A file at the cap needs
    about 2.4 GiB
    ([benchmarks](../benchmarks.md#phase-3-summary-slice-39)).
  - **Past the cap,** a store still exports whole, but its JSON can't be
    imported whole. A streaming import is the way past that, if ever
    needed.
