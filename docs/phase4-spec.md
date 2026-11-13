# Atlas — Phase 4 Implementation Specification v0.1

**Status:** Ready for coding-agent implementation, beginning with Slice 4.1  
**Date:** 2026-11-12 (see Revision notes at the end)  
**Parent:** [Atlas Product Specification v0.2](product-spec.md) §5, §7, §8, §9, §13; builds on the [Phase 3 Implementation Specification](phase3-spec.md), the [Phase 3 close-out review](phase3-closeout.md), [ADR 0001](adr/0001-local-ipc-no-server-single-data-root.md) (the single data root), [ADR 0002](adr/0002-data-root-creation-locking-and-compatibility.md) (creating and locking it), [ADR 0004](adr/0004-resource-policy.md) (the resource policy), [ADR 0008](adr/0008-extraction-and-vector-engines.md) and [ADR 0009](adr/0009-jobs-and-extraction.md) (engines, jobs and the worker), [ADR 0011](adr/0011-re-reading-without-losing-search.md) (re-reading), [ADR 0013](adr/0013-embeddings-durable-index-disposable.md) (the meaning index) and [ADR 0016](adr/0016-the-knowledge-store.md) (the knowledge store and its snapshots)  
**Target platforms:** macOS arm64, Windows x64, Linux x64  
**Product:** Atlas Community — local-first personal knowledge discovery  
**Milestone:** *Full-collection readiness*: Atlas reads the whole collection, scans included, keeps itself current, and can be backed up, checked, moved and recovered

> **Product promise:** Collect freely. Organize minimally. Retrieve intelligently.  
> **Experience rule:** Simple by default; powerful by choice. No AI knowledge or terminal required for ordinary users.

## 1. Mission and definition of done

Phases 0 to 3 built the librarian and its memory, and proved them on fixtures: 30,000 generated files, a 33-document corpus, a knowledge store of 100,000 steps. Four things still stand between that and a lifelong collection. Scanned documents are only labelled, never read. The library goes stale until the user presses Rescan. Nothing can be backed up, checked, repaired or moved from inside Atlas. And when something goes wrong, Atlas keeps no record of it.

Phase 4 closes those gaps and then meets the real collection. The product spec's criterion is the gate: **hundreds of GB can be indexed progressively and recovered after failures without losing user notes.**

Phase 4 is complete **only** when the following holds on each of the three target OSes:

1. **Scanned PDFs are read.** Documents marked Needs OCR are read page by page in the background, in priority order. The work pauses on battery, and after a kill it resumes at the next page.
2. **The library stays current without being asked.** Atlas rescans at launch and on a timer, and a rescan that finds nothing changed is cheap.
3. **The user decides when background work runs:** automatically, only while charging, only between set hours, or not at all; and can ask for full speed now.
4. **Atlas can check and repair itself.** Check library reports on both databases and both indexes; the search index and the meaning index can be rebuilt; the catalog can be compacted. A damaged catalog has a way back that isn't starting over.
5. **Everything can be backed up and restored.** Trails and notes are also copied to a second folder the user chooses. The catalog has a verified backup inside the data folder. Both can be restored inside Atlas, and a restore first saves what is there.
6. **Move Atlas data works**, and a kill at any point leaves exactly one whole library.
7. **Atlas keeps a record.** A local log of counts and timings, and a report the user can save. Nothing is sent anywhere, and neither ever holds a path, a title or a word the user typed.
8. **The real collection.** The product owner runs the release build against their real collection from the checklist in §10: it is indexed progressively, scans included, survives kills, battery and a move, and no trail or note is lost.
9. Unit, integration and smoke tests, benchmarks, packaging and licence evidence are recorded for macOS, Windows and Linux.

**Do not mark Phase 4 complete on builds alone, or on fixtures alone.** Item 8 is the owner's run, and the phase stays open until it is recorded. If an OS is unavailable for a check, document it as unverified with a reproducible checklist, as Phases 0 to 3 did.

### What Phase 4 builds on

- **Measured, in the close-outs and `docs/benchmarks.md`:**
  - 30,000 files are read in 343 to 350 s at the policy's rate on the reference machine.
  - The catalog is 965 MiB for 594 MiB of text, and about 1.7 GB more with vectors at 1,024 dimensions. The meaning index file is 1.5 GB at 733,668 chunks, and rebuilds in about 54 s at a million vectors.
  - A full rebuild of the search index takes about 7.5 s at 30,000 files.
  - A knowledge store of 100,000 steps is 380 MiB and snapshots in 0.72 s on the reference machine (3.5 to 5.1 s on CI runners).
- **Found in the code while planning:**
  - **Needs OCR is detection only, and per document** (`extraction/engine/garble.rs`). The worker streams every page and the app then throws them away; `page_count` stays NULL. A reader upgrade never re-reads these documents (`extraction/db.rs`, `mark_refresh`).
  - **The worker already turns a page into an image** (`engine/render.rs`, protocol 2). The supervisor does not check the protocol number a worker reports.
  - **`jobs.priority` exists and is sorted on, and nothing writes it.** Job kinds are scan, extract and embed.
  - **A rescan writes every row again** (`inventory/db.rs`), so a rescan that changes nothing costs about what a first scan does: 79 to 99 s at 30,000 files across the machines measured. That is the first scan's time; a no-change rescan was never timed.
  - **Atlas has no log.** `logs/` is created and empty. There is no logging crate and no panic hook in the app.
  - **`integrity_check` runs only on staged copies** (`catalog/convert.rs`, `knowledge/snapshot.rs`), never on a live database. There is no restore, no catalog backup and no move: "Use a different location…" opens a separate library.
  - **No performance setting exists**, and no rule reacts to other applications using the CPU.
  - **The fixture has no scanned files.** The spike corpus has two, and one more PDF with unreadable fonts.
- **Already met, so not rebuilt:** index versioning for readers (ADR 0011: a reader version per format, re-reading without losing search) and for embeddings (ADR 0013: a model row per model, dimensions and chunker; generation-named index files). Phase 4 adds an OCR version and nothing else here.
- **The product-owner decisions behind this document (2026-11-12):**
  - **the gate is the owner's real collection**, run by the owner; fixtures decide CI;
  - **signing, notarization and updates stay deferred**;
  - **automatic rescans, no file watcher**;
  - **trails and notes are also backed up to a second folder**; the catalog's backup stays in the data folder; both restore inside Atlas;
  - **the OCR engine is chosen by a measured spike**, by the rule in §10;
  - **OCR is English only**, ships **inside the installer**, and scans are **a small share** of the collection.

## 2. Scope boundaries

### Implement in Phase 4

- **Diagnostics:** a local log of typed events, a panic hook, and a counts-only report.
- **OCR:** the engine the spike chooses, in the worker; an `ocr` job; page-level resume; passages from OCR.
- **Priority:** Read this now, and an order for OCR.
- **Automatic rescans:** at launch, on a timer, and when a drive returns; a rescan that writes nothing when nothing changed.
- **Schedules:** when background work runs; full speed; a rule for a busy computer.
- **Check library and repairs:** checks of both databases and both indexes; rebuild, compact, and removing records of files long gone; recovery from a damaged catalog.
- **Backups and restore:** the catalog's backup; trails and notes to a second folder; in-app restore of each.
- **Move Atlas data:** a verified copy, an atomic switch, a journal.
- **Scale:** scanned fixtures, 100,000-file runs, and the fixes they call for.
- **Benchmarks, CI, packaging and licences** for the above; smoke steps; the owner's real-collection run.

### Explicitly defer

Signing, notarization and in-app updates (their own release-readiness phase); filesystem watchers; OCR of languages other than English; OCR through a vision model on the user's AI service; OCR of partly scanned documents that pass today's rule (they are counted, §7.2); an outline on OCR'd pages in the page view (no positions are stored); compressing stored passages (ADR 0010 stands; the real collection's sizes are recorded in the close-out); a trigram index for Chinese and Japanese; incremental snapshots and a streaming import (measured against the real store, not built); re-linking a library folder that moved, and opening a library on another computer; encryption of backups; an EPUB renderer, text selection and annotations; `atlas://` links; a "Check for newer recommendations" fetch; a reranker, summaries and query rewriting; §7.7 of the Phase 2 spec; an Anthropic native adapter; an Atlas-managed runtime and model downloads; thumbnails and covers; sync of trails and bookmarks (Phase 5); a background service that runs after Atlas quits.

**Scope discipline.** One executable. Two SQLite databases, each with one writer. The OCR engine is loaded only in the worker process, as PDFium is. Atlas writes outside its data root in exactly two places, both chosen by the user in a native dialog: the `Atlas` folder of an export destination (Phase 3) and the `Atlas Backups` folder of a backup destination (§7.7). A Move ends with one data root, not two. No network request is added: Atlas still contacts only the AI service the user configured.

## 3. Technology choices (Phase 4 additions)

| Concern | Phase 4 choice | Notes |
| --- | --- | --- |
| OCR engine | **Decided by Slice 4.2** (ADR 0020), by the rule in §10 | Candidates: Tesseract 5 (Apache-2.0; Leptonica BSD-2), PP-OCRv5 and PP-OCRv6 through ONNX Runtime (models Apache-2.0; runtime MIT; v6 added in Slice 4.2), and Apple's Vision framework on macOS only. `ocrs` is excluded: its models are CC BY-SA 4.0 |
| Page images for OCR | `pdfium-render` 0.9.5 as pinned: `render_with_config` at 300 DPI, greyscale, annotations off | Already in the worker. A page that is one embedded image may be taken at its own resolution instead |
| OCR process | a third instance of the extraction worker's `Supervisor`, for OCR only | After extraction and the page view (ADR 0018). The main process still loads neither PDFium nor the OCR engine |
| Staging OCR pages | a table in the catalog (`ocr_pages`), emptied as each document is committed | So a kill costs one page, and nothing is written under `jobs/checkpoints/` |
| Logging | Atlas's own small writer over a closed set of typed events; JSON lines | No logging crate. A general text logger can't enforce "no words"; a typed event with numeric fields can |
| Timers | one thread per open library, as the knowledge store's hourly ticker | Rescans, checks, backups. Driven by the `Clock` Phase 3 added, so tests move time |
| Checks | `PRAGMA quick_check`, `integrity_check` and `foreign_key_check`; FTS5 `'integrity-check'`; BLAKE3 of the index file | All in SQLite and `blake3`, already dependencies |
| Compact and backup | `VACUUM INTO` a staged file, checked, renamed | The pattern of `catalog/convert.rs` and `knowledge/snapshot.rs`, generalised |
| Verified copies | BLAKE3 while reading the source, and again over the written file | For the second backup folder and Move. `fsutil::atomic_write` and `sync_dir` as today |
| Other applications' CPU use | `sysinfo`, already a dependency | Where a reading is missing the rule doesn't apply, as ADR 0004 treats every gap |
| Dialogs | `tauri-plugin-dialog`, called from Rust only | The backup folder, the move destination, and a snapshot file to restore |

No new dependency is expected outside the OCR engine and its binding. If a slice needs one, it records the licence and the size in the slice's notes.

## 4. Repository layout

Phase 3's layout stands. Phase 4 adds (new files marked *new*):

```text
spikes/ocr-bench/                              # new (4.2): the OCR spike, its corpus manifest and results
fixtures/scans/                                # new (4.3): page images with known text, for scanned fixtures
src-tauri/
  migrations/0015_ocr_pages.sql                # new (4.1): what a Needs OCR document is known to hold
  migrations/0016_ocr.sql                      # new (4.3): OCR staging, versions, passage source
  migrations/0017_requested.sql                # new (4.4): Read this now
  src/services/diagnostics/{mod,event,log,report}.rs          # new (4.1)
  src/services/ocr/{mod,job,db,order}.rs       # new (4.3, 4.4)
  src/services/extraction/engine/ocr.rs        # new (4.3): the engine, in the worker only
  src/services/extraction/{protocol,worker_main,supervisor}.rs   # gain the Ocr request (protocol 3)
  src/services/inventory/auto.rs               # new (4.5): when a rescan is due
  src/services/scheduler/{schedule,busy}.rs    # new (4.6): modes, hours, full speed, the busy rule
  src/services/maintenance/{mod,check,repair,recover,setup}.rs   # new (4.7)
  src/services/backup/{mod,catalog,second,restore}.rs         # new (4.8)
  src/services/workspace/relocate.rs           # new (4.9): Move Atlas data and its journal
  src/commands/{diagnostics,maintenance,backup,relocate}.rs   # new
  benches/{ocr,maintenance}.rs                 # new
  tests/{diagnostics,ocr,ocr_crash,priority,auto_rescan,schedule,
         check,repair,backup,restore,relocate,relocate_crash,lifecycle}.rs   # new
src/
  lib/components/{DiagnosticsPanel,BackgroundWorkPanel,CheckPanel,BackupsPanel,
                  RestoreDialog,MoveDialog,ReadNowButton}.svelte   # new
  lib/stores/{diagnostics,maintenance,backups,relocate}.svelte.ts  # new
  lib/{diagnosticsCopy,ocrCopy,scheduleCopy,maintenanceCopy,backupCopy,moveCopy}.ts   # new
docs/adr/0020-…0025-….md                       # new: OCR; diagnostics; rescans and schedules; checks and repairs; backups and restore; moving the data root
docs/phase4-closeout.md                        # new (4.11)
```

## 5. User experience and screen contracts

Voice, tokens and patterns are those of Phases 0 to 3. New wording lives in copy modules, as before. Nothing here adds a main view.

### 5.1 Scanned documents

- **A document that needs OCR** reads **"Scanned · waiting to be read"** in the Library and the details panel, in place of "Needs OCR". While it is being read: **"Reading scanned pages: 120 of 340"**. With OCR switched off, or for a book Atlas can't OCR (an EPUB with unreadable text), the Phase 1 wording stays.
- **Once read**, the details panel says **"Text read from scanned pages. It may contain mistakes."** and, when some pages failed, "12 pages couldn't be read."
- **A search result or cited passage** from such a page carries the word **"scanned"** beside its place, in words, not only an icon.
- **The indexing line** gains scanned work: "Reading scanned documents: 12 of 240 · about 3,100 pages to go". On battery: "Paused reading scanned documents on battery · Resumes when plugged in".
- **The page view** shows an OCR'd page as before: the image, and now its text beneath. No passage is outlined on it, and the note says why.

### 5.2 Read this now

- **Read this now** is a button in the details panel of a file waiting to be read, a scanned document waiting for OCR, and a document waiting for its meaning to be indexed. It moves that one to the front of its queue and says so: "Next in line."
- It is not offered while background work is **Paused** by the user; the panel says "Background work is paused" with **Resume**.

### 5.3 Staying current

- **Library folders** show when each was last checked ("Checked 2 hours ago") beside Rescan, which stays.
- **Atlas checks folders by itself** at launch and on a timer. A check that finds nothing says nothing. One that finds changes updates the counts as a rescan does today.
- **A folder whose drive is away** says so, as today, and is checked when the drive returns.

### 5.4 Settings › Performance & Battery

The panel keeps its readings and gains:

- **Background work** (a radio group): **Automatic** (the default; today's behaviour), **Only while charging**, **Only between** two times of day, **Paused**. A sentence under each says what it means, and the current state is stated in words ("Running", "Waiting until 22:00", "Waiting for power").
- **Run at full speed now:** a button, offered on mains power. It lasts until the work is done or Atlas quits, and shows **Stop**.
- **Check folders for changes:** Every hour, Every 6 hours (the default), Once a day, Only when I ask.
- **Read scanned documents (OCR):** on by default.

### 5.5 Settings › Storage: checks and repairs

- **Check library** runs the quick check and reports each area in a line: the catalog, your trails and notes, the search index, the meaning index. "Everything checked out" or what was found, in words. **Check thoroughly** runs the full check, with progress, and can be stopped.
- **Repairs**, each with a sentence on what it does and how long it took last time: **Rebuild the search index**, **Rebuild the meaning index**, **Compact the library**, **Remove records of files gone for over 30 days** (with the count).
- **A damaged catalog** opens a recovery screen instead of the library: what happened, that trails and notes are safe, and two choices: **Restore the backup from** a date, or **Start a new catalog** (Atlas reads the folders again; the damaged file is kept).

### 5.6 Settings › Storage: backups and restore

- **Your trails and notes** lists its snapshots (date, size, how many trails, findings and bookmarks each holds) with **Restore…**, and **Restore from a file…**.
- **Also keep copies in…** chooses the second folder. Once set: the folder, when the last copy was made, **Back up now**, **Stop keeping copies there**. When the folder can't be reached: "Atlas can't reach the backup folder. 3 snapshots are waiting."
- A sentence says what a copy is: "Copies hold your searches, questions and notes, unencrypted. If the folder syncs to a cloud service, they go there too."
- **The catalog** shows its backups (date, size), **Back up now**, a switch **Keep a backup of the catalog** (on), and **Restore…**.
- **Restore** confirms in words what will be replaced and that Atlas saves what is here first: "Trails and notes added since 3 November will be replaced. Atlas keeps a copy of what is here now."

### 5.7 Move Atlas data

- **Move Atlas data…** sits in Settings › Storage beside "Use a different location…", which keeps its meaning. A sentence tells them apart.
- **The dialog:** the size to move, the destination (chosen in the native dialog), whether it fits, and **Move**. Then progress in bytes with **Cancel**, the check, and the result: "Atlas now keeps its library at … . The old copy is still at … (3.1 GB)."
- **Remove the old copy…** appears in Storage until it is used or dismissed, and names the folder and the size.
- **If Atlas was closed during a move**, the next launch says which library it opened and why.

### 5.8 Diagnostics

A small section in Settings:

- **Save a diagnostics report:** shows what the report holds and what it never holds, then writes it and offers **Show in Finder** (or the platform's wording).
- **Show the log folder**, the log's size, and **Clear the log**, which asks first (Slice 4.1.1).
- A sentence: "Atlas keeps a log of what it did, in counts and timings, on this computer. Nothing is sent anywhere."

### Visual and accessibility rules

As Phases 0 to 3. Every long task (a thorough check, a compact, a backup, a restore, a move) has a determinate progress bar where the total is known, a polite status line, and a result stated in a sentence. Progress is never the only signal: the state is also text. Destructive confirmations name what goes and how much. Times of day use native time inputs. "scanned" is a word, never only an icon.

## 6. Data root, lifecycle and persistence

### 6.1 The catalog

**Migration 0015 (Slice 4.1):** what a Needs OCR document is known to hold.

```sql
ALTER TABLE documents ADD COLUMN text_pages INTEGER;   -- pages with a usable text layer, in a document that needs OCR
```

From 4.1, a document that needs OCR keeps its `page_count` and its `text_pages`. Documents marked before 4.1 have neither, and are read once more to get them (§7.9).

**Migration 0016 (Slice 4.3):** OCR.

```sql
ALTER TABLE documents ADD COLUMN ocr_version INTEGER;                  -- NULL: never read by OCR
ALTER TABLE documents ADD COLUMN ocr_failed_pages INTEGER NOT NULL DEFAULT 0;
ALTER TABLE passages  ADD COLUMN source TEXT NOT NULL DEFAULT 'text'
    CHECK (source IN ('text', 'ocr'));

CREATE TABLE ocr_pages (                 -- pages finished for a document still being read
    document_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    page_no     INTEGER NOT NULL,
    state       TEXT NOT NULL CHECK (state IN ('done', 'failed')),
    attempts    INTEGER NOT NULL DEFAULT 0,
    text        TEXT,                    -- NULL when failed, or when the page is blank
    reason      TEXT,                    -- a fixed code, never a message
    PRIMARY KEY (document_id, page_no)
) STRICT, WITHOUT ROWID;
```

`jobs.kind` gains `ocr` (the column has no CHECK; `catalog/codec.rs` does).

**Migration 0017 (Slice 4.4):** Read this now.

```sql
ALTER TABLE file_instances ADD COLUMN requested_at TEXT;   -- before the file has a document
ALTER TABLE documents      ADD COLUMN requested_at TEXT;   -- for OCR and for embedding
```

**Rescans (Slice 4.5)** are expected to need no migration: the rows a scan saw are held in a temporary table, not written to each row.

### 6.2 The data root

```text
<ATLAS_DATA_ROOT>/
  config/setup.json            # new (4.7): a mirror of what the user set up (§7.6)
  backups/catalog/             # new (4.8): catalog backups, and staging/
  backups/knowledge/           # as Phase 3; a restore first adds a snapshot of what it replaces
  exports/diagnostics/         # new (4.1): reports the user saved
  logs/                        # used from 4.1: atlas-<date>.jsonl
```

`LAYOUT_VERSION` stays 1: every addition is a folder or file an older Atlas ignores. `content/extracted`, `assets/thumbnails`, `jobs/checkpoints` and `models` stay reserved and empty.

**Outside the data root:**

- `locator.json` (OS config folder), as today.
- `move.json` beside it (new, 4.9): the journal of a move in progress. It is outside both roots because either may be unreachable at launch.
- `<chosen>/Atlas Backups/` (new, 4.8), §7.7.

### 6.3 Settings

`config/settings.json` gains, with defaults that keep old files valid:

- `background.mode`: `automatic` | `charging` | `hours` | `paused` (default `automatic`); `background.from` and `background.to` (local times, default 22:00 and 07:00).
- `background.checkFolders`: `hourly` | `sixHours` | `daily` | `never` (default `sixHours`).
- `background.ocr`: true.
- `backups.catalog`: true.
- `backups.secondFolder`: kept beside the `Settings` DTO, as `export.lastFolder` is. The renderer never receives or sends it as a path it can change; it is set only through Rust's dialog.

### 6.4 The knowledge store

No schema change. Its snapshots (§6.3 of the Phase 3 spec) are unchanged; Phase 4 adds copying them out and restoring them.

## 7. Behaviour

### 7.1 Diagnostics (`services/diagnostics/`)

- **An event is a code and numbers.** `Event` is a closed enum. Its fields are integers, durations, booleans, and values from other closed enums (`JobKind`, `PauseReason`, `ErrorCode`, a command's name). It has no field that can hold a string made at run time. That is the privacy rule, enforced by the type.
- **What is recorded:**
  - Atlas starting and stopping; a library opening, with its schema versions; migrations.
  - Each job starting, pausing, resuming and finishing, with its counts and its time.
  - Workers starting and crashing, by code.
  - Each policy decision that changes.
  - Snapshots, backups, restores, checks, repairs and moves, with their outcomes.
  - Errors returned to the UI, by `ErrorCode` and command.
  - Searches and questions as a duration and a count of results or sources. Never their words.
  - A panic, as a source location.
- **The log** is JSON lines in `logs/atlas-<UTC date>.jsonl`. A file rolls at 5 MiB; the folder is capped at 50 MiB and the oldest file goes first.
- **Writing it** is a dedicated thread behind a bounded queue. A full queue drops events and counts them. A log that can't be written never fails or slows the work it describes. Nothing is logged while no library is open.
- **The report** (`report.rs`) is one JSON file, written to `exports/diagnostics/atlas-diagnostics-<time>.json`:
  - versions (Atlas, the OS, the schema of both databases, the readers, the chunker, OCR);
  - the device readings and the policy's decision now;
  - the settings that are modes and switches;
  - storage by category;
  - counts: files by state and by storage, their total size by format, documents by state, documents and pages waiting for OCR, passages, chunks, vectors per model, jobs by kind and state, trails, steps, bookmarks, findings and collections;
  - snapshots and backups (how many, how large, when);
  - the last check's results;
  - the last 2,000 events.
- **It never holds** a path, a file or folder name, a title, an author, a search, a question, an answer, a note, an excerpt, a URL or a key. Models are counted by position, never named (Slice 4.1: a service's label is free text the user typed).
- **Storage** counts the log under its own line. Clean caches leaves it; Clear the log removes it.

### 7.2 OCR (`services/ocr/`, `extraction/engine/ocr.rs`)

- **What is read.** A PDF in the state `needs_ocr`:
  - with the reason `no_text_layer`, the pages with under 20 visible characters are read by OCR and the others keep their text layer;
  - with the reason `unreadable_text`, every page is read by OCR.
- **What is not.** A PDF that passed the Phase 1 rule but has some image-only pages is left as it is; from 4.1 the reader counts those pages, and the report gives the total. An EPUB marked unreadable has no pages to OCR and stays as it is.
- **The request.** `Ocr { path, page, dpi }`, protocol 3. The worker renders the page with PDFium, runs the engine, and answers `OcrPage { page, text, … }` or `OcrFailed { page, reason }`. One page per request.
- **The job.** A job of kind `ocr` per library folder, queued when a read leaves documents needing OCR and at open. Its runner is a fourth thread per library. It:
  1. takes the next document in order (§7.3);
  2. reads its text-layer pages through `Extract`, when it has any;
  3. asks for each remaining page, writing each answer to `ocr_pages` in its own transaction;
  4. commits the document.
- **The commit** is one transaction, through the existing `commit_document`: passages in page order, `source = 'ocr'` on those from OCR, `state = 'done'`, `ocr_version`, `ocr_failed_pages`, the reason cleared, and the document's `ocr_pages` rows deleted. The search index follows by its triggers, and the embedding job finds the document as it finds any newly read one.
- **Limits.**
  - A page has 60 s. A worker that goes quiet or passes its memory cap is killed and restarted, as for extraction.
  - A page is tried twice, then marked failed with a code.
  - Text from a page passes the same readability test as a text layer; a page that fails it counts as failed. A blank page is done, with no passage.
  - A document whose every page failed becomes `failed` with the reason `ocr_failed`.
  - A file that changed since the document's pages were staged loses them and waits to be read again.
- **Policy.** OCR pauses on battery, as embedding does, and under every pause of ADR 0004. It runs at the duty cycles extraction uses. It never runs while `background.ocr` is off.
- **Versions.** `OCR_VERSION` is a constant beside the reader versions. Raising it marks OCR'd documents for a re-read by the ADR 0011 pattern: they stay searchable until the new text replaces the old, and unchanged chunks keep their vectors.
- **Copies** of a scanned file link to its document by content hash, as copies of any file do, and are not read twice.
- **Language.** English. A scan in another language is read as if it were English and its text will be poor. Atlas does not detect this; it is a documented limit.

### 7.3 Priority (`services/ocr/order.rs`, the extraction and embedding queues)

- **Read this now** sets `requested_at` on the file or the document, and raises its folder's job (`jobs.priority = 1`) so that job runs before other folders' jobs. The extraction, OCR and embedding queues each take requested items first, oldest request first. `requested_at` is cleared when the item is done.
- **OCR's order** after requests:
  1. documents the user has kept or used: bookmarked, in an interest or project, or opened in a trail;
  2. the rest, fewest pages first, so that more documents become searchable sooner.
- **What the user kept** is read from the knowledge store when a pass starts and held in memory. It is never written to the catalog, which still holds nothing of what the user did.
- **A requested item runs outside the schedule** (§7.5): the user asked. It still stops for every hard pause, and it does not run while background work is Paused.
- Interactive work (search, questions, the page view, opening) is unchanged and never waits on a job.

### 7.4 Automatic rescans (`services/inventory/auto.rs`)

- **When a folder is checked:**
  - at launch, once the library is open, if its last complete scan is older than the interval;
  - on the timer, at the interval in `background.checkFolders`;
  - when a folder that was unreachable becomes reachable;
  - never while that folder already has a scan queued or running, and never when the setting is "Only when I ask".
- **An automatic rescan is an ordinary scan job**, under the resource policy and the schedule, pausable and cancellable. It has no special path.
- **A rescan writes only what changed.** An entry whose size and whole-second modification time match its row is noted in a temporary table and its row is left alone. New, changed, moved and cloud-state changes are written as today. When a scan completes, rows not noted are marked missing, exactly as today: only after a complete scan, never under a folder that failed to list, never when the folder is unreachable.
- **A rescan that found changes** queues reading, OCR and embedding as a manual one does.
- **Moves** are unchanged: a moved file is a new row linked to its document by content hash, and the old row goes missing. Anchors (Phase 3) already follow the document.

### 7.5 Schedules and the busy rule (`services/scheduler/`)

The policy's order becomes:

1. **Hard pauses**, as ADR 0004: the data drive missing, low space, critical memory or heat, battery at or under 20%.
2. **Paused by the user** (`background.mode = paused`).
3. **The schedule:** in `charging` mode, pause unless on mains power or the computer has no battery; in `hours` mode, pause outside the window (which may cross midnight).
4. **Slowing**, as ADR 0004: on battery, power unknown, elevated pressure, and now **a busy computer**.
5. Otherwise run.

- **A busy computer.** When processes other than Atlas and its workers use more than 60% of all cores, averaged over 30 s, background work slows; it returns when that falls under 40%. The thresholds are provisional and are measured in Slice 4.6. Where the reading is unavailable the rule does not apply.
- **Full speed** is held in memory, never in settings. While it is on, the duty cycle is 1 and the scan rate is unthrottled. Hard pauses still apply, it is not offered on battery, and it ends when the queues are empty, on Stop, or when Atlas quits.
- **New reasons** are added to `PauseReason` (`PausedByUser`, `OutsideHours`, `NotCharging`) and `ThrottleReason` (`OtherApps`), each with its sentence in the UI.
- Pausing one job by hand (Phases 0 to 3) is unchanged and survives restarts.

### 7.6 Checks, repairs and recovery (`services/maintenance/`)

- **The quick check** runs weekly in the background, as a low-priority task under the policy, and on Check library:
  - `PRAGMA quick_check` on the catalog and on the knowledge store;
  - the meaning index file's BLAKE3 against the one recorded.
- **The thorough check** adds `integrity_check` and `foreign_key_check` on both databases and FTS5's `'integrity-check'`.
- **A check only reads.** Its result is a list of areas, each `ok`, `problem` (with a fixed code) or `skipped`, kept for the report.
- **Repairs:**
  - **Rebuild the search index:** FTS5 `'rebuild'`, from the stored passages.
  - **Rebuild the meaning index:** the index file is discarded and built again from the stored vectors (ADR 0013).
  - **Compact the library:** `VACUUM INTO` a staged file at 16 KiB pages, checked, then swapped in with the jobs stopped. It needs free space of 1.2 times the catalog beyond the low-space reserve. A kill leaves the old catalog. This is also how a catalog still at 4 KiB pages is converted, whatever its state.
  - **Remove records of files gone for over 30 days:** the file rows, and documents left with no file, except documents the knowledge store refers to, so that an anchor still says "This file isn't in your library any more".
- **The setup mirror.** `config/setup.json` holds what the user set up and the catalog alone knows: the library folders, the AI service connections (never a key) and the model roles. It is rewritten whenever one changes.
- **A damaged catalog.** When the catalog fails to open, or a check finds it damaged, the library opens in recovery (§5.5) with the knowledge store untouched and readable. **Start a new catalog** moves the damaged file to `backups/catalog/catalog-damaged-<time>.sqlite`, creates a catalog, restores the folders, connections and roles from the mirror, and queues a scan. Trails and notes resolve again as documents are read (`unknown`, then `found`, as P3-08 showed).
- **A damaged knowledge store** offers its newest snapshot (§7.7) in place of today's pointer to a folder.

### 7.7 Backups and restore (`services/backup/`)

- **The catalog's backup.**
  - **When:** weekly, if a reading, OCR or embedding job has finished work since the last one; on mains power; with free space of 1.2 times the catalog beyond the low-space reserve; and on Back up now.
  - **How:** `VACUUM INTO backups/catalog/staging/`, `quick_check`, an atomic rename to `catalog-<UTC time>.sqlite`. It reads through a snapshot, so jobs keep running.
  - **Kept:** two. The older goes only after a newer one has passed.
  - It holds the text and the vectors. The meaning index file is not backed up: it is rebuilt from the vectors.
  - It protects against damage and mistakes, not against losing the disk. The UI says so.
- **Trails and notes in a second folder.**
  - **Choosing it:** Rust's folder dialog. Refused: a folder inside the data root or inside a library folder, one Atlas can't write to, and one where `Atlas Backups` exists as a file or a symlink. A folder on a network or cloud-synced volume is allowed: only finished files are placed there.
  - **Where:** `<chosen>/Atlas Backups/<the first 8 characters of the root id>/`, with `atlas-backup.json` naming the root and the Atlas version.
  - **When:** after each snapshot passes its check, and on Back up now.
  - **How:** the snapshot is copied to a temporary name there, flushed, read back and compared by BLAKE3, then renamed.
  - **Kept:** the newest 14 files whose names Atlas's pattern matches. Nothing else in the folder is read, changed or deleted.
  - **Unreachable:** reported in Storage; the copies are made when it returns.
- **Restoring trails and notes.**
  - **From:** a snapshot in the list (those in `backups/knowledge/` and, when reachable, those in the second folder), or a file chosen in Rust's dialog.
  - **The file is untrusted.** It is opened read-only, must be an Atlas knowledge store of a version this Atlas supports, and must pass `integrity_check`. Otherwise it is refused whole.
  - **Then:** the current store is snapshotted (or, if it is damaged, moved to `backups/knowledge/knowledge-damaged-<time>.sqlite`), the writer stops, the file is copied in and renamed over the store, the store reopens and migrates if it was older.
  - **A restore replaces.** To merge instead, the Phase 3 export and import are the way, and the dialog says so.
- **Restoring the catalog:** from a backup in `backups/catalog/`. The jobs stop, the file is swapped in, the catalog migrates, the meaning index is rebuilt if it no longer matches, and every folder is rescanned, so files that changed since the backup are read again.
- **Never touched** by Clean caches, by low-space handling or by any job, as Phase 3's snapshots. Low space stops new backups and says so.

### 7.8 Move Atlas data (`services/workspace/relocate.rs`)

- **The destination** is chosen in Rust's dialog and must be an empty or new folder on a local volume, writable, outside every library folder and outside the current data root, with room for what is copied plus a tenth, beyond the low-space reserve. On battery the dialog asks for power first.
- **The steps**, each recorded in the journal (`move.json`: both paths, the root id, the state):
  1. **Stop.** Every job runner stops, both databases are checkpointed and closed. The old root's lock is kept.
  2. **Copy** (`copying`). The destination gets a marker, `.atlas-moving`; a folder with it is never opened as a library. Every file is copied except `cache/`, `tmp/`, staging folders and the lock. Each is hashed while read and again once written. Cancel is offered until this step ends.
  3. **Check** (`copied`). The copied databases are opened read-only: `integrity_check` on the knowledge store, `quick_check` on the catalog.
  4. **Switch** (`switched`). The marker is removed, then the locator is written atomically.
  5. **Reopen** from the new root. If that fails, the locator is written back and the old root is opened.
  6. **Retire the old copy.** Its `atlas.json` is renamed `atlas.moved.json`, so it is no longer a library, and its lock is released. The journal is removed.
- **After a kill**, the next launch reads the journal:
  - `copying` or `copied`: the old root is the library. A destination that still has the marker and the journal's root id is removed; anything else is left and reported.
  - `switched`: the new root is the library, and step 6 is finished.
- **Remove the old copy** deletes, under the old root, only the files and folders of Atlas's layout, then the folder if it is empty. It is always the user's action.
- **A failure while copying** (no space, the drive gone, a hash that doesn't match) removes the partial copy and leaves the old library open, with the reason.
- **The root id is unchanged**, so the second backup folder and the keychain entries stay valid. At every moment exactly one folder with that id is a library.
- **Library folders are untouched.** A move to a disk another computer will use is out of scope: the catalog's folder paths belong to this computer (ADR 0003).

### 7.9 The carry-overs (Slice 4.1)

- **The supervisor checks the worker's protocol.** A worker that reports another `protocol` than the app's is refused, with a clear error. Today any `Hello` is accepted.
- **Needs OCR documents keep their page counts.** The PDF reader reports `page_count` and `text_pages` for a document it marks Needs OCR. Documents marked earlier, which have neither, go back to waiting once and are read again. They aren't searchable, so nothing is lost while they are.
- **Image-only pages are counted** in documents that pass the rule, for the report.
- **The P3-18 flake.** The log records when the meaning index is loaded and dropped; the cause of empty first suggestions is looked for with it, and recorded whether or not it is found (Slice 4.1: one cause found and fixed, one not; see the revision notes).

## 8. Privacy and security

- **The log and the report hold counts and timings only** (§7.1). The Phase 3 rule stands and now has something to guard: P4-02 searches both for marker words placed in file names, titles, text, searches, questions and notes.
- **Nothing is sent.** A report is a file the user may choose to share. Atlas has no crash reporter and no telemetry, and Phase 4 adds no network request.
- **A second backup folder holds the most personal data Atlas has**, unencrypted. The UI says so where the folder is chosen, and says that a syncing folder sends the copies wherever it syncs. Atlas never chooses such a folder itself.
- **OCR text is document text:** untrusted, delimited in prompts and subject to the citation checks of ADR 0015, exactly as text from a text layer. OCR mistakes can make a quote check fail; the answer then says so, as it does for any passage.
- **OCR of untrusted PDFs** stays in a killable process with a timeout and a memory cap. The main process loads neither PDFium nor the engine.
- **A file to restore is untrusted:** opened read-only, never attached to another database, size-checked, and refused unless it is a knowledge store this Atlas supports and passes its check.
- **Paths still come only from dialogs Rust shows:** the backup folder, the move destination, the file to restore. The renderer sends ids and choices.
- **Move** copies inside the user's own permissions and deletes nothing without the user's action. **Remove the old copy** and the second folder's retention delete only files Atlas wrote.
- **Keys** stay in the keychain. They are not in the setup mirror, a backup, a report or the log.

## 9. Service interfaces and extension-ready design

```rust
// Earlier traits stand. Phase 4 adds:
enum Event { /* closed; numeric and enum fields only */ }
struct Diagnostics;            // record(Event); the log writer; report(&Handles) -> Report
trait OcrEngine { fn read(&mut self, page: &PageImage) -> Result<OcrText, OcrError>; }   // in the worker only; a fake in tests
struct OcrJob;                 // the fourth runner; one page per request; stages, then commits
struct Schedule;               // mode, hours, full speed; decide(now, device) -> Decision, before ADR 0004's slowing rules
trait CpuProbe { fn other_apps(&self) -> Option<f32>; }                                  // sysinfo in the app; a fake in tests
struct Check;  enum Area { Catalog, Knowledge, SearchIndex, MeaningIndex }
enum Repair { SearchIndex, MeaningIndex, Compact, PurgeMissing }
struct Backups;                // catalog backups; the second folder; restore with a dry-run token, as import
struct Relocation;             // plan(dest) -> MovePlan; run with progress and cancel; resume(journal) at launch
```

New typed IPC commands (each registered in the slice that implements it; `tests/ipc_contract.rs` forbids stubs):

```text
get_diagnostics_status() -> DiagnosticsStatus                        # 4.1: log size, last report
save_diagnostics_report() -> ReportSaved;  reveal_diagnostics() -> ()   # 4.1: written under exports/diagnostics
clear_log() -> ()                                                    # 4.1
get_extraction_overview(...)                                         # 4.3: gains OCR's documents and pages
read_now(target) -> ()                                               # 4.4: a file id or a document id
update_settings(...)                                                 # 4.5, 4.6: gains background.*; 4.8: backups.catalog
set_full_speed(on) -> ResourceStatus                                 # 4.6
check_library(depth) -> CheckReport;  cancel_check() -> ()           # 4.7
repair_library(repair) -> RepairReport                               # 4.7
get_recovery() -> RecoveryState;  start_new_catalog() -> ()          # 4.7
get_backups() -> BackupStatus;  back_up_now(what) -> BackupOutcome   # 4.8
choose_backup_folder() -> BackupStatus;  forget_backup_folder() -> BackupStatus   # 4.8: Rust shows the dialog
preview_restore(source) -> RestorePreview;  apply_restore(token) -> RestoreReport # 4.8: source is a listed id or "file"
preview_move() -> MovePlan;  start_move(token) -> ();  cancel_move() -> ()        # 4.9: Rust shows the dialog
get_move_status() -> MoveStatus;  remove_old_copy() -> ()            # 4.9
```

**DTOs.** All ts-rs exported; `check-bindings` covers them.

**Events.** `ocr-progress`, `maintenance-progress` and `move-progress`, each coalesced, carrying counts and bytes only.

**Security boundary.** As Phase 3: the renderer sends ids and choices, never a path. Every long task takes a token from a dry run where it replaces or moves data (`apply_restore`, `start_move`), as `apply_import` does, so that what the user confirmed is what runs.

## 10. Evaluation

### 10.1 The OCR spike (Slice 4.2)

`spikes/ocr-bench/`, in the manner of Slice 0.9 (`docs/spike-0.9.md`):

- **The corpus:** about 50 English pages with reference text, fetched by a script with pinned hashes, never committed where a licence forbids it:
  - 20 pages of book scans (the 1894 *Alice* already in the spike corpus, and other public-domain scans whose text Project Gutenberg holds);
  - 15 pages of scanned two-column papers;
  - 10 degraded pages: low resolution, skew, speckle, JPEG artefacts;
  - 5 pages that already carry a poor OCR text layer.
- **Measured, per engine, at 200 and 300 DPI:**
  - character and word error rates after normalising case, whitespace and hyphenation;
  - reading order, by the measure Slice 0.9 used;
  - seconds per page on one thread, peak memory, and the time to the first page in a new worker;
  - what the engine adds to each installer, and to a clean CI build on each OS;
  - whether the bundle still passes the worker checks in each installed layout.
- **The rule:**
  - **Tesseract is chosen** unless PP-OCR's word error rate over the corpus is at least 3 points lower, each installer grows by at most 40 MiB, and it builds and passes the worker checks on all three OSes. Then PP-OCR is chosen. PP-OCR's entry is the best of its v5 and v6 variants within those limits (Slice 4.2; ADR 0020 defines the measures and the variants).
  - **Apple's Vision framework is used on macOS** only if its word error rate is at least 3 points lower than the chosen engine's there. If it is used, ADR 0020 records that text read on macOS differs from text read elsewhere.
  - **If no candidate builds on all three OSes**, the slice stops and reports; OCR does not ship on some OSes only without the product owner's decision.
- **The result** is ADR 0020 and `docs/spike-ocr.md`, with every measurement.

### 10.2 Scanned documents are found

`tests/ocr.rs` reads the scanned fixtures (§11) and the spike corpus's scans with the chosen engine on every OS, and searches for phrases from their reference text. The share found is recorded, and its floor is set from the spike's result before Slice 4.3 starts. Phase 2's and Phase 3's evaluation floors (P2-17, P3-13) must not fall.

### 10.3 The real-collection run (the gate)

Run by the product owner on the release build, from a checklist in `docs/platform-validation.md`. The coding agent never opens the owner's library; the evidence is what the owner reports and the diagnostics reports the owner chooses to share.

1. **A report at the start,** from the library as it is.
2. **Days of ordinary use** while Atlas reads, OCRs and indexes in the background under **Automatic**, then under a schedule.
3. **Failures on purpose:** Atlas force-quit during OCR and during a rescan; the laptop unplugged; the lid closed overnight.
4. **Staying current:** files added, moved and deleted in Finder are picked up without Rescan.
5. **A scanned book is found** by words from inside it, and a question is answered from it with a citation.
6. **Backups:** the second folder holds verified copies. A snapshot from a week earlier is restored, and then the newest is restored again.
7. **A move** to an external disk and back, with a force-quit during one of them.
8. **Check library** passes at the end, and every trail, bookmark and finding from before the run is still there.
9. **A report at the end.**

Drills that would put the real library at risk (a damaged catalog, a full disk, Start a new catalog) are run on a scratch library built from the fixtures, by the same checklist.

## 11. Test fixture collection

- **Scanned fixtures.** `fixtures/scans/` holds about 20 page images with known text, each with a marker phrase. `scripts/generate-fixtures.mjs` gains `--scanned-every N`: it wraps those images into image-only PDFs, in differing orders so that each file's hash differs, and records each file's phrases.
- **A mixed PDF:** text pages and scanned pages in one file, flagged by the rule, for P4-09.
- **The 100,000-file fixture** is the existing generator at a larger size, for the rescan and maintenance benchmarks. The 30,000-file fixture stays the standing asset.
- **Damaged databases** are made by the tests from good ones: a page overwritten, an index entry removed, a foreign key broken. None is committed.
- **Fakes:** an `OcrEngine` that returns a page's known text (so most OCR tests don't need the engine); the `CpuProbe`; the settable clock; the power and volume fakes of earlier phases. Debug-only hooks answer the three new dialogs, as `ATLAS_E2E_PICK_EXPORT_DIR` does.
- **The OCR spike's corpus** stays in `spikes/`, fetched on demand.

## 12. Required automated tests

| ID | Scenario | Expected result |
| --- | --- | --- |
| P4-01 | The log through a session: open, scan, read, embed, search, a question, a worker crash; a file reaching 5 MiB; the folder reaching 50 MiB; a log folder that can't be written | Typed events with counts and timings; files roll and the oldest goes; the work succeeds whether or not the log can be written |
| P4-02 | Marker words in file names, titles, text, searches, questions, answers and notes, through a full recorded session and a saved report | No marker in `logs/` or in the report (extends P3-16) |
| P4-03 | The report | Written only under `exports/diagnostics/`; valid against its schema; its counts equal the catalog's and the knowledge store's |
| P4-04 | A worker reporting another protocol; documents marked Needs OCR before 4.1 | The worker is refused with a clear error; the documents are read once more and gain page counts; nothing else is re-read |
| P4-05 | Scanned fixtures and the corpus's scans read by OCR | Passages with `source = 'ocr'`; phrases from the reference text found by search; the documents `done`; the main process loads neither PDFium nor the engine |
| P4-06 | SIGKILL of the app during a document, and of the worker during a page | The document resumes at the first page not done; no finished page is read again; `integrity_check` passes |
| P4-07 | A page that hangs; one that fails twice; a blank page; a worker over its memory cap; a document whose every page fails | The page is skipped with a code and the document says how many failed; the last becomes `failed` with `ocr_failed`; a running extraction job is undisturbed |
| P4-08 | OCR on battery, under low space and switched off; a file that changes mid-document | It pauses and keeps its finished pages; it resumes; the changed file's staged pages are dropped and it waits to be read |
| P4-09 | A mixed PDF; `OCR_VERSION` raised | Text-layer pages keep their text and only the others are read by OCR; the re-read keeps the document searchable and embeds only changed chunks |
| P4-10 | Read this now on a waiting file, a scanned document and one waiting for its meaning; kept documents; the rest | Requested first, then kept, then fewest pages; the folder's job runs before others; nothing about what was kept is written to the catalog; a request runs outside the schedule and not while Paused |
| P4-11 | A rescan of an unchanged folder; then one new, one changed, one moved, one deleted and one cloud-only file | The first writes no row; the second detects each exactly as the Phase 1 and Phase 3 tests expect |
| P4-12 | Automatic rescans: at launch, by the timer (settable clock), a returning drive, the setting off, a folder already being scanned, under a pause | One scan queued in each due case and none otherwise; never two scans of one folder; the policy is obeyed |
| P4-13 | Each mode of Background work, with the clock and power fakes; a window across midnight; a desktop with no battery | The decision and reason §7.5 names; a desktop is never "waiting for power" |
| P4-14 | Full speed; the busy rule | Duty 1 until the queues empty, on Stop or at quit; hard pauses still stop it; other applications' CPU slows work and it returns under the lower threshold |
| P4-15 | Check library on a healthy library and on copies with a damaged catalog page, a damaged knowledge store, a search index out of step, an altered index file and a broken foreign key | All `ok`; then each reported under its area with its code; no check writes |
| P4-16 | Each repair; SIGKILL during Compact; a catalog at 4 KiB pages | Search results equal before and after; a kill leaves the old catalog whole; the catalog ends at 16 KiB; documents an anchor refers to survive the purge |
| P4-17 | A catalog that won't open | The recovery screen; trails and notes readable; Start a new catalog keeps the damaged file, restores folders, connections and roles from the mirror and queues a scan; the knowledge store is untouched |
| P4-18 | The catalog's backup: due and not due; on battery; under low space; a kill during one; one that fails its check; the third | No partial file; two kept; the older removed only after a newer one passed |
| P4-19 | The second folder: a copy made and verified; unreachable then back; `Atlas Backups` as a file and as a symlink; a folder inside the data root and inside a library folder; the user's own files there; the fifteenth copy | Copied, or refused with a reason; waiting copies made on return; nothing Atlas didn't write is read, changed or deleted |
| P4-20 | Restoring trails and notes from a snapshot and from a file; a damaged file, a newer one and a file that isn't Atlas's; a kill during the swap | The current store is saved first; the store equals the snapshot; a bad file is refused and nothing changes; a kill leaves the old store or the new one, whole |
| P4-21 | Restoring the catalog from a backup after files changed | It opens and migrates; the rescan reads the changed files; the meaning index is rebuilt; anchors resolve |
| P4-22 | A move with jobs running | The trees are equal by hash; the locator names the new root; jobs resume there; the old copy is no longer a library; Remove the old copy removes only Atlas's files |
| P4-23 | SIGKILL in each state of a move (`copying`, `copied`, `switched`, during retirement) | At the next launch exactly one whole library opens, the one the journal names; a partial destination is removed only when it carries the marker and the root id |
| P4-24 | A move to a network volume, a synced folder, a non-empty folder and a library folder; too little space; the destination disappearing; a hash mismatch; Cancel | Refused, or rolled back with the reason; the old library is open and unchanged |
| P4-25 | Settings files from 0.4.0; migrations 0015 to 0017; a catalog from a newer Atlas | Old files are valid with the defaults; each migration applies once; the newer catalog is refused unaltered |
| P4-26 | **The lifecycle,** on a 1,000-file fixture with 50 scanned files: scan, read, OCR, embed, an automatic rescan, a backup, a check, a move, a restore | Every count as expected at each stage; originals unchanged throughout |

Use the injected fakes (§11). Add integrity checks on both databases after every crash test, and the originals-unchanged snapshot to every test that reads files, as before. Tests that need the real OCR engine are P4-05 and the spike; the rest use the fake engine.

## 13. Benchmarks and hardware validation

Record performance, don't invent guarantees. At minimum measure:

- **OCR** (`benches/ocr.rs`, the scanned fixtures and the spike corpus): pages per second at the Run and Slow duty cycles and unthrottled, the worker's peak memory, the time to the first page, and search latency while OCR runs.
- **Rescans** (`benches/inventory.rs`): a rescan that changes nothing at 30,000 and at 100,000 files, with the bytes it writes; one that changes 1%.
- **Maintenance** (`benches/maintenance.rs`, the 30,000-file library with vectors): the quick and thorough checks; each repair; the catalog's backup; a restore of each database; a move, in bytes per second, to the same disk and to an external one where the machine has one.
- **The log:** throughput of reading 30,000 files with and without it.
- **Scale:** the 100,000-file fixture cataloged and read, with the catalog's size and the app's peak memory.
- **The real collection,** from the owner's reports: files, bytes, documents by state, pages read by OCR, the catalog's size, and how long each stage took.
- macOS reference machine first; Windows and Linux CI evidence before sign-off.

Provisional targets, to report against and not to publish as requirements: **a no-change rescan of 30,000 files in under 10 s**, writing under 1 MiB; **the log costs under 1%** of reading throughput; **the quick check in under 60 s** and **the catalog's backup in under 5 minutes** at 30,000 files with vectors; **a restore of trails and notes in under 5 s** at 100,000 steps; **search p95 while OCR runs** within Phase 1's target. OCR's own speed has no target until the spike has measured it.

## 14. Cross-platform packaging, CI and release evidence

- **Licences.** `docs/dependency-licenses.md` gains the OCR engine, its models and its binding; model licences are recorded as carefully as code licences. GPL/AGPL and share-alike findings are flagged.
- **Bundles.** Record the growth since 0.4.0 in `docs/packaging.md`. The engine and its English model ship in every installer; each installer may grow by at most 40 MiB. The engine's native library, if it has one, is placed and checked as PDFium is.
- **Pinned downloads.** Anything fetched at build time (engine source, models) is pinned by hash in `tools.lock.json`, as PDFium is.
- **CI.** The full tier runs the new tests with the fake engine everywhere and P4-05 with the real one. `spikes.yml` gains the OCR spike. The bench job adds `benches/ocr.rs` and `benches/maintenance.rs`. The worker checks in the installed layouts gain "the worker reads one scanned page".
- **Smoke** (Windows and Linux, after Phase 3's steps): "a scanned PDF is read and found", "a diagnostics report is saved", "Check library passes", "the library is moved to a scratch folder and reopens", "a snapshot is restored".
- **Release evidence.** The real-collection run (§10.3) on macOS by the product owner, and its scratch-library drills, per the checklist in `docs/platform-validation.md`.
- The artifacts still start without Python, Docker, a database server, Ollama or an LLM. Without an AI service, OCR, rescans, checks, backups, restore and Move all work.

## 15. Implementation backlog: ordered vertical slices

Implement and verify **one slice at a time**. Build and rerun tests after each slice, then proceed. Never claim later slices are done because scaffolding compiles.

| Slice | Deliverable | Verification gate |
| --- | --- | --- |
| 4.1 | **Diagnostics and carry-overs.** The log, the panic hook, the report, the Diagnostics section; the protocol check; page counts for Needs OCR documents (migration 0015); slice strings to Phase 4; ADR 0021 | P4-01, P4-02, P4-03, P4-04. The product owner then saves a report from the real library; its counts are recorded and size Slices 4.3 to 4.5 |
| 4.2 | **The OCR spike.** `spikes/ocr-bench/`, the corpus, the measurements on three OSes, the decision by §10.1's rule; ADR 0020; `docs/spike-ocr.md` | Every §10.1 measurement recorded for every candidate; a clean build and worker check on three OSes for the engine chosen; licences recorded |
| 4.3 | **OCR.** The engine in the worker, `Ocr` (protocol 3), the `ocr` job, staging and commit (migration 0016), the policy, the scanned fixtures, the wording of §5.1 | P4-05, P4-06, P4-07, P4-08, P4-09; `benches/ocr.rs`; the floor of §10.2 set and met |
| 4.4 | **Priority.** Read this now (migration 0017), `jobs.priority` in use, OCR's order, OCR progress in the indexing line | P4-10; Vitest |
| 4.5 | **Automatic rescans.** The rescan that writes only changes; launch, timer and returning drives; "Checked … ago"; ADR 0022 | P4-11, P4-12; the no-change rescan benchmarked at 30,000 files |
| 4.6 | **Schedules.** Background work's modes, hours, full speed, the busy rule, the settings; ADR 0022 completed; ADR 0004 amended | P4-13, P4-14, P4-25 (settings); Vitest |
| 4.7 | **Check library and repairs.** The checks, the four repairs, the setup mirror, recovery from a damaged catalog; ADR 0023 | P4-15, P4-16, P4-17; Vitest |
| 4.8 | **Backups and restore.** The catalog's backup, the second folder, restore of both databases; ADR 0024 and ADR 0001's amended list | P4-18, P4-19, P4-20, P4-21; Vitest |
| 4.9 | **Move Atlas data.** The plan, the verified copy, the journal, the switch, the old copy; ADR 0025 (amends ADR 0002) | P4-22, P4-23, P4-24; the smoke's move step on Windows and Linux |
| 4.10 | **Scale.** The lifecycle test; `benches/maintenance.rs`; the 100,000-file runs; fixes for what they find; the release candidate the owner's run starts on | P4-26; §13's measurements on the reference machine; the owner's run started and its first report recorded |
| 4.11 | **Evidence and close-out.** Benchmarks on three OSes; the licence report; the owner's run finished and recorded; the scratch drills; `docs/phase4-closeout.md`; version 0.5.0 | All CI green on `main`; §10.3 recorded; review against §1, §12, §13, §14 and §16 |

**Ordering notes.** 4.1 is first: every later slice records what it does, and the owner's first report sizes the OCR work. 4.2 comes before any OCR code. 4.4 needs 4.3. 4.5 and 4.6 need neither OCR slice and can move earlier if the spike is slow. 4.8 needs 4.7 (a restore ends with a check). 4.9 needs 4.8's verified copy. Slices 4.7 to 4.9 must be done before the owner's run starts in 4.10: the run should not begin on a library that can't be backed up.

Each slice includes: code, automated tests, a README and docs update (`architecture.md`, the ADRs, `benchmarks.md`, `platform-validation.md`, `packaging.md` and the justfile as affected), explicit demo steps, and a brief note of any deviations from this specification. Checkpoints use the established convention: a `slice-4N` branch, squash-merged onto `main` as `phase4/slice-0N: …` (`phase4/slice-10`, `phase4/slice-11`) with an annotated tag of the same name.

## 16. Acceptance checklist for the product owner

- [ ] A scanned book in my collection is found by words from inside it, and Atlas can answer from it with a citation.
- [ ] Scanned documents are read in the background without my laptop getting hot or the battery draining, and what I ask for is read first.
- [ ] Files I add, move or delete show up in Atlas without my pressing Rescan.
- [ ] I can tell Atlas to work only while charging, only overnight, or not at all, and I can tell it to hurry.
- [ ] Check library tells me in plain words whether everything is sound, and the repairs work.
- [ ] My trails and notes are copied to a second folder I chose, and I restored them from a copy.
- [ ] When the catalog was damaged (on a scratch library), I got my library back without losing a trail or a note.
- [ ] I moved Atlas's data to another disk and back, and force-quitting in the middle cost me nothing.
- [ ] I can save a report of what Atlas has been doing, and it holds nothing I wouldn't share.
- [ ] My real collection, hundreds of GB, was indexed progressively over days and survived being killed, unplugged and moved.
- [ ] Recorded test, benchmark, licence and platform-smoke evidence for the three OSes.

## 17. Constraints for the coding agent

1. **Read this entire spec, the Phase 3 spec, the Phase 3 close-out review, ADRs 0001, 0002, 0004, 0009, 0011, 0013 and 0016, and Atlas Product Specification v0.2 first.** The parent spec is product direction; this document is the Phase 4 scope and acceptance contract.
2. Work incrementally, beginning with Slice 4.1. Deliver working code rather than architecture-only commentary. Prefer maintained existing libraries over custom replacements, except where this spec says why not (the log).
3. If implementation conflicts with a stated invariant (read-only originals, recoverable jobs, no mandatory AI, no silent network calls, keys only in the keychain, the single data root with its documented exceptions), preserve the invariant and document the tradeoff.
4. **What the user creates is never deleted to recover space, never rebuilt from something else, and never the casualty of a repair, a restore, a move, a cache clean, a job or a migration.** A restore saves what is there first. A move deletes nothing by itself.
5. **Every operation that replaces or moves a database is staged, checked, then swapped,** and a kill at any point leaves a whole database: the old one or the new one. Each has a SIGKILL test.
6. **Atlas writes outside its data root only** inside the `Atlas` folder of an export destination and the `Atlas Backups` folder of a backup destination, each chosen in a native dialog, and never inside a library folder. There it changes and deletes only files it wrote.
7. **No free text in the log or the report.** An event's fields are numbers and closed enums. If something can't be said that way, it isn't logged.
8. **The OCR engine and PDFium load only in the worker process.** Never in the main process, never in the renderer.
9. Don't overbuild future phases. No watcher, no signing, no updater, no other OCR languages, no vision-model OCR, no encryption of backups, no cross-computer move, no compression of passages.
10. Validate inputs on the Rust side. Treat document text, OCR text, model output, restored files and names as untrusted data. The renderer sends ids and choices, never paths.
11. Use explicit migrations for both databases and structured error codes. Prefer deterministic fakes (the settable clock, the fake OCR engine, the CPU, power and volume fakes, the scripted service) to real waiting, real engines or real models.
12. Never show a document as read before its transaction committed, a backup as made before it passed its check, or a move as done before the new root opened.
13. Run formatting, static checks and tests before claiming each slice is done. Report commands executed and outcomes. Never claim a Windows, Linux or macOS build was tested unless it actually ran there; label unverifiable steps.
14. When a measurement contradicts a document, correct the document in the same change.
15. **Never read the owner's API keys, never download a model at run time, never open the owner's real library, its knowledge store, its backups or its log.** Measurement runs use scratch libraries built from the corpus and fixtures. The real collection's evidence is what the owner reports and the diagnostics reports the owner shares.
16. **Commits and pushes** (the owner's standing rule): commit freely on slice branches; push branches, `main` and `phase*` tags once the full CI run is green; never bypass commit signing, retry a failed signature, then stop and ask; ask before any force-push or deletion of a branch or tag.
17. End each slice report with: implemented, tests passed, manual verification, known issues, and next slice.

## 18. Kickoff instructions for the coding agent

**First requested task:** Implement **Slice 4.1** only:

- Add `services/diagnostics/`: the `Event` enum, the log writer (JSON lines in `logs/`, 5 MiB files, a 50 MiB cap, a bounded queue), the panic hook in the app, and the report written to `exports/diagnostics/`.
- Record events at the points §7.1 names, starting with library open, jobs, workers, policy changes, snapshots and errors returned to the UI.
- Add the Diagnostics section to Settings, the log's line in Storage, and the commands `get_diagnostics_status`, `save_diagnostics_report`, `reveal_diagnostics` and `clear_log`.
- Make the supervisor refuse a worker whose `Hello.protocol` differs from the app's.
- Add migration 0015; have the PDF reader report `page_count` and `text_pages` for a document it marks Needs OCR, and count image-only pages in documents that pass; send earlier Needs OCR documents back to waiting once.
- Add P4-01, P4-02, P4-03 and P4-04, and the migration test for 0015.
- Change the slice strings to Phase 4 (`CURRENT_SLICE = "4.1"`, the test mocks and `ipc_contract.rs`).
- Write ADR 0021 (diagnostics); amend ADR 0017 ("Atlas has no log"); update `docs/architecture.md`, `migrations/README.md`, the README status and the justfile as affected.
- Ask the product owner to save a diagnostics report from the real library with the CI-built 0.4.x DMG, and record its counts: files, bytes, documents needing OCR and their pages, the catalog's size.

After the slice passes, summarize code and verification, note deviations, and name Slice 4.2 as the next step.

Do **not** add an OCR engine, an `ocr` job, a rescan timer or any backup code before Slice 4.1's gate passes.

---

### Revision notes

**2026-11-12, Slice 4.2, before measuring** (the OCR spike's criteria;
[ADR 0020](adr/0020-ocr-engine.md), Proposed):
- **PP-OCRv6 is measured beside v5**, by the product owner's decision: it
  was released in June 2026 under Apache-2.0. The better of the two within
  §10.1's limits is PP-OCR's entry. v6 has no English-only recogniser, so
  its variants use the multilingual one. The v5 server and v6 medium
  models are excluded before measuring: their models alone pass 40 MiB.
- **The rule's definitions are fixed in ADR 0020**: the normalisation; WER
  and CER by Levenshtein (substitutions count once); the corpus figure as
  the mean of the pages' WERs; 300 DPI; PP-OCR must win on every OS where
  both engines ran; a page over 60 s fails a variant.
- **Recorded, not deciding**, by the product owner's decisions: a 95%
  interval for each WER difference (a paired bootstrap over pages), and
  speed. No speed bound beyond §7.2's 60 s a page.
- **The owner's own scans** (`just spike-ocr-mine`), read by every engine
  and timed, side by side with the page image: a check on behaviour, not
  part of the rule.
- **Reference text for book scans** comes from Wikisource's validated page
  transcriptions where Project Gutenberg's isn't aligned to the page or
  the edition. Wikisource text is CC BY-SA, so it is fetched by a pinned
  script and not committed.
- **The worker check in 4.2 is a stand-in**: a small program built with
  the engine, bundled into a real installer beside Atlas, finding its
  library and models by the worker's rules from its installed place, and
  reading one scanned page. The real worker with the engine inside is
  §14's check in Slice 4.3. ONNX Runtime is Microsoft's official build,
  pinned and loaded at run time, as PDFium is.

**2026-11-12, Slice 4.1.3** (asked by the owner while indexing the real
library):
- **"Going slowly" says why.** The status lines for cataloging, reading
  and indexing meaning name the reason, as their pauses already did:
  "Slowed on battery", "Going slowly · Can't tell if it's plugged in" or
  "Going slowly · The computer is busy" (elevated memory pressure or
  heat). The cataloging line said nothing at all when the computer was
  busy, and the meaning line never named the battery. One function,
  `slowedText`, gives all three their words.

**2026-11-12, Slice 4.1.2** (the Explore spike; [results](spike-explore.md)):
- **The product owner asked for visual ways to browse**: a semantic
  explorer, with books clustered by meaning into named areas on a map and
  in a list, and a view of what the library is made of. The Phase 3 spec
  deferred "graph views, tags and inferred topics"; the product spec lists
  browsing by inferred topics for the Library.
- **Decided by the owner:** a measured spike first, run on the real
  collection by its owner, then a decision. Area names come from the
  books' own words, computed locally; names from the chat model are
  optional and later. Search and trails on the map, a walk from one book
  and bridges between areas were not chosen for now.
- **The spike is inserted before the OCR spike** and changes nothing in
  the app: a module of pure functions (`services/explore`) and a harness
  the owner runs. Building the feature is not Phase 4 scope until the
  spike's rule says go and a specification is written; §17's "don't
  overbuild future phases" stands.
- **Hand-written, with no new dependency**, where the guideline prefers
  maintained libraries: the reason is in the spike's write-up.
- **`catalog::reader::open_offline`** is new: a catalog opened from
  outside Atlas to read only, never migrated, refused at any schema but
  the build's.

**2026-11-12, Slice 4.1.1** (found by the owner on the real library):
- **A meaning job that stopped for good says why.** Such a job isn't
  active, so the status line read "Meaning waiting" and offered nothing.
  A folder's summary now carries its latest job's code while that job is
  the failed one. The status line and the folder's line show the reason,
  with Open Settings when the key, the model or the address has to change
  and Index meaning now otherwise.
- **`job_ended` carries the reason** of a failed job, as a closed code
  (`FailureReason`; a code it doesn't know is `other`). The report's
  version stays 1: the field is optional and older lines read as before.
- **Clear the log asks first** (§5.8): "Clear the log? Its events can't be
  brought back.", with Clear and Keep, as removing a folder does.

**2026-11-12, Slice 4.1** (diagnostics and carry-overs; [ADR 0021](adr/0021-diagnostics.md)):
- **Errors returned to the window are logged by code, not by command.**
  Tauri 2.12 gives no hook on a command's response, and naming the command
  would mean wrapping all 79. They are recorded in `Serialize for
  AtlasError`, the one place an error meets the window.
- **`reveal_diagnostics` takes a folder** (`reports`, which shows the newest
  report, or `log`), so one command serves Show in… and Show the log folder.
- **Models are counted by position, never named**, in the report: a
  service's label is free text the user typed.
- **Image-only pages in documents that pass the rule are counted when the
  report is written**, from the stored passages, by the reader's own rule
  (`garble::is_empty_page`). Every document of a library read before 4.1
  would otherwise be uncounted. The count includes blank pages, and the
  report says so.
- **`text_pages`** counts pages with at least 20 visible characters under a
  tenth garbled: for an `unreadable_text` PDF, a page whose text is garbled
  doesn't count. The protocol stays 2; the counts ride in the `failed`
  frame's two optional fields, as `boilerplate_chars` rode in `done`.
- **A refused worker is `Unavailable`, not a crash**, so it isn't restarted;
  the job ends `failed_terminal` with `worker_protocol`, and the status
  line, which had no words for a reader that can't start, now has them.
- **P3-18 was empty first suggestions, not an answer.** One cause was found
  while adding the meaning index's events: `VectorRegistry::loaded` used
  `try_lock` on the lock `get_or_open` takes, so whenever the embedding job
  fetched its index, suggestions and meaning search saw "not loaded". The
  index and the lock an open holds are now apart. **Another remains:** the
  second P3-18 test's first call came back empty once in a full local run
  after the fix, and not in 14 runs since. It now waits, as the first test
  has since 3.9, and prints the index's state if it fails.
- **Found by CI, fixed: an embedding job could finish with re-read books
  waiting.** The job walks documents in rowid order; one read again behind
  it waited again while reading's trigger saw the job still running. It
  now looks once more from the start for documents the model never tried
  (ADR 0014's amendment). P3-13 caught it on the macOS runner.
- **The log is attached when the library's lock is held**, before the
  catalog migrates, so no buffering of early events is needed.

**2026-11-12, v0.1** (written after the Phase 3 close-out review):

- **Product-owner decisions recorded:**
  - the gate is the owner's real collection, run by the owner from a checklist; fixtures decide CI;
  - signing, notarization and updates stay deferred, for a release-readiness phase of their own;
  - automatic rescans at launch and on a timer, with no file watcher, where the product spec (§5) names watchers;
  - trails and notes also copied to a second folder the owner chooses; the catalog backed up inside the data folder; in-app restore of both;
  - the OCR engine chosen by a measured spike, by a rule written here;
  - OCR in English only, shipped inside every installer, with scans a small share of the collection.
- **Decided in writing this spec, for the product owner to confirm or change:**
  - **OCR in the existing worker binary** under a third supervisor, one page per request, with finished pages staged in the catalog so a kill costs one page.
  - **Only documents marked Needs OCR are read by OCR.** Partly scanned documents that pass the Phase 1 rule are counted, not read.
  - **The spike's rule** favours Tesseract, the product spec's named engine and the smaller one, unless PP-OCRv5 is clearly more accurate within 40 MiB an installer. Apple's Vision is used on macOS only if it clearly beats the choice.
  - **No outline on OCR'd pages:** the engine's positions aren't stored.
  - **OCR's order** after requests: what the user kept or opened, then the fewest pages first. What was kept is read from the knowledge store and never written to the catalog.
  - **A log of typed events, written by Atlas's own code,** not a logging crate, so that "no words" is enforced by the type.
  - **The report is written inside the data root** (`exports/diagnostics/`), so it adds no exception to ADR 0001.
  - **A second exception to ADR 0001:** `Atlas Backups` in a folder the owner chose. A syncing folder is allowed there, with the disclosure of §5.6.
  - **Two catalog backups, weekly,** not daily: ADR 0016 judged a daily copy of a multi-GB catalog too expensive.
  - **A restore replaces** and saves the current store first; merging stays export and import.
  - **A mirror of what the user set up** (`config/setup.json`), so a new catalog doesn't lose the folders, connections and roles.
  - **Move** copies and verifies, switches the locator, and keeps the old copy until the owner removes it. The root id doesn't change.
  - **Schedules** are four modes and a full-speed button; no CPU budgets, concurrency settings or quiet-hours calendar.
  - **The busy rule's thresholds** (60% and 40% over 30 s) are provisional.
  - **Index versioning adds only an OCR version.** Readers and embeddings already have theirs.
  - **Passages are not compressed.** ADR 0010's reasoning stands, and the Phase 3 spec's note that Phase 4 would do it is withdrawn pending the real collection's sizes.
  - **Eleven slices,** two more than Phase 3, because the product spec gives this phase the most separate capabilities.
- **Carried from the Phase 3 close-out and still deferred:** incremental snapshots and a streaming import (measured against the real store in 4.10, not built); the page view's missed outlines where no piece of a passage matches; signing and notarization; the unexplained miss of an index's first vector on the Linux runner; the 30k fixture embedded through a real service (the real collection's run supersedes it).

### Primary reference documentation (verify current versions at implementation time)

- Tesseract (quality, resolution, models): <https://tesseract-ocr.github.io/tessdoc/>, <https://tesseract-ocr.github.io/tessdoc/ImproveQuality.html>
- PP-OCRv5 models: <https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_rec>; PP-OCRv6: <https://huggingface.co/PaddlePaddle/PP-OCRv6_small_rec_onnx>; ONNX Runtime: <https://onnxruntime.ai/docs/>
- Apple Vision text recognition from Rust: <https://docs.rs/objc2-vision/latest/objc2_vision/>
- pdfium-render (page rendering): <https://docs.rs/pdfium-render/0.9.5/pdfium_render/>
- SQLite `quick_check`, `integrity_check` and `foreign_key_check`: <https://www.sqlite.org/pragma.html>; `VACUUM INTO`: <https://www.sqlite.org/lang_vacuum.html>
- FTS5 `'integrity-check'` and `'rebuild'`: <https://www.sqlite.org/fts5.html>
- `sysinfo` (per-process CPU): <https://docs.rs/sysinfo/latest/sysinfo/>
- Tauri dialog plugin (Rust API): <https://docs.rs/tauri-plugin-dialog/latest/tauri_plugin_dialog/>
