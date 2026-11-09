# Atlas — Phase 1 Implementation Specification v0.1

**Status:** Ready for coding-agent implementation  
**Date:** 2026-11-08 (see Revision notes at the end)  
**Parent:** [Atlas Product Specification v0.2](product-spec.md); builds on the [Phase 0 Implementation Specification](phase0-spec.md), the [Phase 0 close-out review](phase0-closeout.md) and [ADR 0008](adr/0008-extraction-and-vector-engines.md)  
**Target platforms:** macOS arm64, Windows x64, Linux x64  
**Product:** Atlas Community — local-first personal knowledge discovery  
**Milestone:** A *useful local library*: every book and paper searchable by its own text, without AI

> **Product promise:** Collect freely. Organize minimally. Retrieve intelligently.  
> **Experience rule:** Simple by default; powerful by choice. No AI knowledge or terminal required for ordinary users.

## 1. Mission and definition of done

Phase 0 built the foundation: a desktop shell that catalogs PDFs and EPUBs read-only into one SQLite database inside a chosen data root, browses and opens them, respects battery and storage, survives interruption, and tests an AI endpoint without using it. Phase 1 makes that catalog **useful on its own**: Atlas reads the text of every document it can, in the background and under the same policy as cataloging, and lets the user find a book or paper by what it says. It recognises the same content stored twice, shows honestly which files it could not read and why, and does all of this while the library stays browsable and the originals stay untouched. AI remains optional; nothing in Phase 1 needs a model.

Phase 1 is complete **only** when the following holds on each of the three target OSes:

1. After cataloging, Atlas reads the text of PDFs and EPUBs in the background and shows a per-file status: **text ready**, **waiting**, **reading**, **needs OCR**, **encrypted**, **only in the cloud**, or **couldn't read**, each with a reason the user can understand.
2. Full-text search over the extracted text answers the committed evaluation corpus (§10) with the known-relevant document **in the top 5 results for at least 80% of the questions**.
3. On the 30,000-file fixture generated with body text (§11), **100% of well-formed files** are extracted; every file that is not (malformed, encrypted, scanned, cloud-only) has a per-file status and reason, never a silent skip. On the spike corpus, every document's outcome matches its manifest's `expect`.
4. Rescans re-read only files whose size or modification time changed. After a full extraction pass, the originals' bytes, modification times and hashes are unchanged (a snapshot test proves it).
5. Extraction slows on battery, pauses at ≤ 20% battery, on low data-root space, when the data root or a source root disappears, and when the app quits; it resumes without re-doing finished files and never leaves a partially written document.
6. Search stays responsive while extraction runs: 95th-percentile `search_documents` latency under about 250 ms on the 30,000-file fixture with body text on the reference machine (reported, not asserted, on CI runners).
7. Files with identical content are recognised as one document, extracted once, and shown with all their locations; a moved file keeps its text.
8. Unit, integration and smoke tests, benchmarks, packaging with PDFium, and license evidence are recorded for macOS, Windows and Linux.

**Do not mark Phase 1 complete based on builds alone if installation and extraction have not been exercised on a platform.** If an OS is unavailable, document it as unverified with a reproducible checklist, as Phase 0 did.

The two numbers in the parent spec's Phase 1 done criterion ("an agreed share of questions", "an agreed share of the fixture") were set by the product owner on 2026-11-08: **top 5 for ≥ 80%**, and **100% of well-formed files with every other file reported**.

## 2. Scope boundaries

### Implement in Phase 1

- **A general jobs table and runner.** `scan_sessions` becomes `jobs` with a `kind`; extraction is the second job kind, as Phase 0 §6 required. Pause, resume and cancel work for both kinds; the Phase 0 state machine is unchanged.
- **Text extraction in a helper process:** PDFium through `pdfium-render` for PDF, `rbook` + `html2text` for EPUB, exactly as ADR 0008 decided, run as a second mode of the Atlas executable with a timeout, a memory cap and automatic restart. Signature sniffing (file content, not just extension). The ADR's rules: detect unreadable text and flag it for OCR; guard EPUB input (DRM, entry sizes); never open cloud-only files.
- **Content identity:** a BLAKE3 hash of each file's bytes; a logical **document** per distinct hash; file instances link to documents. Duplicates are extracted once and shown as one document with several locations; moved files keep their text.
- **Per-file extraction status** in the Library, a status-bar line for extraction, and job controls for both kinds.
- **Full-text index and search:** SQLite FTS5 over extracted passages, a query builder that accepts terms, `"phrases"` and a trailing `*`, ranked results grouped by document with a snippet and the page or chapter, merged with Phase 0's file-name matching.
- **Search UI and document detail:** Home's search box searches text; a Search view shows results; a detail panel shows title, authors, pages, text status with reason, and duplicate locations.
- **Metadata:** title, authors and language from PDF Info and EPUB `dc:` metadata, with the file name as fallback.
- **Fixtures with body text** and valid EPUBs; extraction and search benchmarks on the 1,000- and 30,000-file sets; the retrieval evaluation harness and its committed questions.
- **Indexing estimate and storage:** usage by category (catalog, text index, caches), a sampling-based estimate of remaining text volume, and cache cleanup.
- **Phase 0 carry-overs:** the cost of resuming inside a very large folder; Windows junctions and paths over 260 characters; a worker that never outlives the app.
- **Packaging:** PDFium bundled on all three OSes with its third-party notices, hash-pinned and signed; CI, smoke and license evidence extended.

### Explicitly defer

An OCR engine (Phase 1 only flags **needs OCR**; the product spec schedules OCR for Phase 4); cover thumbnails; filesystem watchers and periodic rescans (rescans stay explicit); Move Atlas Data; embeddings, semantic search, related documents, chat, RAG and citations (Phase 2); interests, projects, trails, bookmarks, Markdown export and Obsidian integration (Phase 3); index versioning, backups, integrity tools and diagnostics (Phase 4); a second extractor such as Docling, or any Python; model downloads; an embedded PDF reader; public HTTP API; user authentication; cloud sync; encryption beyond OS protections; a background daemon after quitting; third-party plugin loading; app auto-updates; installer signing and notarization for public distribution.

**Scope discipline:** Provide stable seams for later work, but do not introduce a general framework or service because it *might* be useful. Prefer one executable (in two modes), one SQLite database, and traits only where a test injects a fake. Never run a separate database server.

## 3. Technology choices (Phase 1 additions)

Phase 0's choices stand. Phase 1 adds:

| Concern | Phase 1 choice | Notes |
| --- | --- | --- |
| PDF text | `pdfium-render` =0.9.5 with `default-features = false`, feature `pdfium_8125`; PDFium chromium/8125 from bblanchon/pdfium-binaries | Shared library, hash-pinned in `tools.lock.json`, bundled per OS. **`thread_safe` stays off**: the worker is single-threaded and process-isolated, which is also how the spike measured it |
| EPUB text | `rbook` =0.7.12 + `html2text` =0.17.4 | Spine order; one passage per chapter piece |
| EPUB pre-scan | `zip` (the version `rbook` depends on, `default-features = false`) | Checks `mimetype`, `META-INF/encryption.xml`, entry names and uncompressed sizes before `rbook` reads anything |
| Content hash | `blake3` | Streaming, in the worker, from the same read as extraction |
| Full-text index | SQLite FTS5, external-content table over `passages`, `tokenize='unicode61 remove_diacritics 2'`, `detail='full'` | Already compiled into the bundled SQLite (`SQLITE_ENABLE_FTS5`); no new dependency |
| Worker protocol | JSON lines over stdin/stdout, `serde_json` | One request at a time; streamed page frames |
| Memory cap | `sysinfo` (already a dependency) polling the worker's RSS | Kill and restart over the cap |
| UI | No new frontend dependencies | Snippets are typed spans, never HTML |

**Rule:** Verify current package compatibility, minimum Rust version and transitive licenses before pinning; never use floating versions in the committed configuration. PDFium's `LICENSE` and `licenses/` folder ship with the app and appear in the license report. All documentation must reflect what was implemented rather than imagined functionality.

## 4. Repository layout

Phase 0's layout stands. Phase 1 adds (new files marked *new*):

```text
tools.lock.json                              # new: pinned PDFium downloads per OS (SHA-256)
scripts/fetch-pdfium.mjs                     # new: verifies and unpacks into src-tauri/vendor/pdfium/<platform>/
fixtures/evaluation/questions.json           # new: the committed retrieval evaluation corpus
src-tauri/
  tauri.macos.conf.json                      # new: bundle.macOS.frameworks = [vendor/pdfium/darwin-arm64/libpdfium.dylib]
  tauri.windows.conf.json                    # new: bundle.resources for pdfium.dll and notices
  tauri.linux.conf.json                      # new: bundle.resources for libpdfium.so and notices
  vendor/pdfium/<platform>/                  # gitignored; fetched by `just setup` and CI
  migrations/0007_jobs.sql                   # new (Slice 1.1)
  migrations/0008_documents.sql              # new (Slice 1.3)
  migrations/0009_passages_fts.sql           # new (Slice 1.4)
  src/main.rs                                # --extract-worker dispatch before atlas_lib::run()
  src/services/jobs/                         # new: JobRunner, JobProcessor, jobs db, control
  src/services/inventory/scan.rs             # new: ScanProcessor (Phase 0's runner body)
  src/services/extraction/                   # new: job, db, protocol, supervisor, worker_main, engine/{sniff,hash,pdf,epub,garble}
  src/services/search/                       # new: query builder, search, snippets
  src/services/documents/                    # new: detail, duplicates, metadata
  src/commands/{extraction,search,documents,storage}.rs   # new
  benches/extraction.rs                      # new
  benches/search.rs                          # new: latency and --eval
  tests/{jobs,extraction,extraction_crash,search,search_eval}.rs   # new
src/
  views/Search.svelte                        # new
  lib/components/{DocumentDetail,ExtractionBadge}.svelte   # new
  lib/stores/{search,extraction}.svelte.ts   # new
docs/
  phase1-spec.md                             # this document
  adr/0009-jobs-and-extraction.md            # new (Slice 1.1, final in 1.3)
  adr/0010-full-text-search.md               # new (Slice 1.4)
  adr/0011-re-reading-without-losing-search.md   # new (Slice 1.6)
  phase1-closeout.md                         # new (Slice 1.8)
```

The spike harness in `spikes/` stays as it is; its corpus becomes Phase 1's extraction test input, and its `tools.lock.json` keeps `protoc` and `typst` while the PDFium entry moves to the root lock file. Keep `docs/` filenames stable; record versions and dates inside the files.

## 5. User experience and screen contracts

### 5.1 Home

- The search field searches **text and names**. Replace the Phase 0 help text ("This is not AI search yet") with what the search does: "Searches the text of your books and papers, and their names. Use quotes for an exact phrase."
- Submitting opens the **Search** view with the results. Library progress and the folder panel stay as in Phase 0.
- While extraction is still running, Home says so in one line ("Atlas is still reading text: 12,400 of 30,006 files so far. Search covers what's been read.").

### 5.2 Search

- Results are grouped **by document**, best match first. Each result shows: title (or file name when no title is known), type, folder, **where** (page *n* for PDFs; chapter and part for EPUBs), a **snippet** with the matched terms highlighted, and **why** it matched (in the text, in the name, or both).
- Highlighting is rendered from typed spans (`SnippetSpan { text, hit }`); the UI never receives or renders markup.
- Filters: folder, type. Paging over the grouped results. Open and Reveal on each result, with the same cloud-only and missing-file handling as the Library.
- Empty states: no matches ("Nothing in your library mentions …; Atlas has read the text of N of M files"); nothing extracted yet ("Atlas hasn't read any text yet"); a query that is only punctuation.
- Phrase search: `"harmonic drive"` matches the exact phrase (accent- and case-insensitive). A trailing `*` matches a prefix (`meteor*`). Other FTS syntax typed by the user is treated as plain text, never as operators.

### 5.3 Library

- A new **Text** column with a badge: **Text ready**, **Waiting**, **Reading…**, **Needs OCR**, **Encrypted**, **Only in the cloud**, **Couldn't read**. A **Text status** filter next to Folder and Type.
- Selecting a row opens a **document detail** panel: title, authors, language, pages, size, modified, folder and path, text status with its reason in plain words ("This PDF has no readable text layer. It looks scanned; Atlas can't read it without OCR."), **Also at** (other locations of the same content), Open and Reveal.
- Name search in the Library toolbar keeps its Phase 0 behaviour and label ("Search file names and folders").
- Empty state and partial results during scanning are unchanged.

### 5.4 Indexing status

- The status bar gains a second line for extraction, in the same voice as cataloging: **"Reading text · 12,400 of 30,006 files"**, **"Reading text · Slowed on battery"**, **"Paused reading text · Resumes at 25% or when plugged in"**, **"Text up to date · 29,870 of 30,006 files readable"** (the difference is the count of files that need OCR, are encrypted, cloud-only or unreadable, each visible in the Library filter).
- Pause, Resume and Cancel act on the job they are next to; both jobs can be paused independently. Cancel stops the job and keeps every extracted document.
- The per-folder line gains **Read text now**, which enqueues an extraction job for that folder (idempotent).
- Counts are of files processed, never a percentage of an unknown total while cataloging is still discovering files. Reason codes stay separate from display text.

### 5.5 Settings › Storage

- Usage by category: catalog (`catalog.sqlite` and WAL), text index (passages and FTS), caches (`cache/`, `tmp/`), and free space on the data volume.
- The **indexing estimate**: after sampling, "Text for the remaining 17,600 files will need about 1.5–2.5 GB." Always a range, never a point; "Not enough read yet to estimate" before sampling.
- **Clean caches** removes rebuildable caches only (`cache/`, `tmp/`), never the catalog, text or settings, and reports what it freed.

### 5.6 Settings › Performance & Battery

- The policy sentence covers both jobs: "When Atlas catalogs your folders or reads their text, it will …". The battery rule and the provisional note are unchanged.
- Advanced (debug builds only, environment variables): worker count (1 or 2) and unthrottled extraction, mirroring Phase 0's `ATLAS_INVENTORY_UNTHROTTLED`.

### Visual and accessibility rules

As Phase 0: whitespace, restrained typography, one primary action per screen, system fonts, keyboard order, readable status labels, system light/dark mode, no alerts for routine progress, and a recovery action on every error. The Text badge must not rely on colour alone.

## 6. Data root, lifecycle and persistence

The Phase 0 data-root layout and lifecycle rules stand. Phase 1 keeps extracted text **in the catalog** (`library/catalog.sqlite`): one source of truth, transactional with the file instances, and FTS5 snippets need the text in the database. `content/extracted/` stays reserved (ADR 0010 records the reasoning). `jobs/checkpoints/` stays unused: every extraction checkpoint is a row.

### 6.1 Schema additions

```text
0007_jobs.sql (Slice 1.1)
  ALTER TABLE scan_sessions RENAME TO jobs;                      -- SQLite rewrites scan_directories' FK
  ALTER TABLE jobs ADD COLUMN kind TEXT NOT NULL DEFAULT 'scan';  -- no CHECK: see below
  DROP INDEX scan_sessions_by_root;  DROP INDEX scan_sessions_by_state;
  CREATE INDEX jobs_by_root ON jobs (source_root_id, kind);
  CREATE INDEX jobs_by_kind_state ON jobs (kind, state, priority, created_at);
  -- The scan counters stay as columns (0 for other kinds). scan_directories.session_id and
  -- file_instances.last_seen_session_id keep their names: a rename buys nothing.

0008_documents.sql (Slice 1.3)
  documents (id TEXT PRIMARY KEY, content_hash BLOB NOT NULL UNIQUE,
             state TEXT NOT NULL CHECK (state IN ('pending', 'running', 'done', 'needs_ocr',
                    'encrypted', 'malformed', 'unsupported', 'too_large', 'worker_crashed', 'failed')),
             reason TEXT, extractor_version INTEGER, page_count INTEGER, char_count INTEGER,
             title TEXT, authors TEXT /* JSON array */, language TEXT,
             created_at TEXT NOT NULL, updated_at TEXT NOT NULL) STRICT;
  CREATE INDEX documents_state ON documents (state);
  -- Then the file_instances columns, after documents exists:
  ALTER TABLE file_instances ADD COLUMN extract_state TEXT NOT NULL DEFAULT 'pending'
    CHECK (extract_state IN ('pending', 'hashing', 'linked', 'skipped_cloud_only',
                             'unreadable', 'hash_failed'));
  ALTER TABLE file_instances ADD COLUMN hashed_size INTEGER;
  ALTER TABLE file_instances ADD COLUMN hashed_mtime_secs INTEGER;
  ALTER TABLE file_instances ADD COLUMN document_id TEXT REFERENCES documents (id);
  ALTER TABLE file_instances ADD COLUMN extract_error TEXT;
  CREATE INDEX file_instances_extract_queue
    ON file_instances (source_root_id, availability, extract_state);

0009_passages_fts.sql (Slice 1.4)
  passages (id INTEGER PRIMARY KEY, document_id TEXT NOT NULL REFERENCES documents (id) ON DELETE CASCADE,
            ordinal INTEGER NOT NULL, page_no INTEGER, location TEXT, text TEXT NOT NULL,
            UNIQUE (document_id, ordinal)) STRICT;
  CREATE VIRTUAL TABLE passages_fts USING fts5 (text, content='passages', content_rowid='id',
            tokenize='unicode61 remove_diacritics 2', detail='full');
  -- The three AFTER INSERT / AFTER DELETE / AFTER UPDATE triggers from the SQLite FTS5
  -- documentation keep the index in step. The index is never maintained by hand.
```

Semantics:

- **Identity.** A *file instance* is `(source_root_id, rel_path)` with its Phase 0 UUID. A *document* is the BLAKE3-256 of a file's bytes. The document carries the extracted text, its state and the metadata; the instance carries what blocks it before hashing (`skipped_cloud_only`, `unreadable`, `hash_failed`). The **effective status** of a file is its instance state if that blocks, else its document's state.
- **Invalidation.** The scan upsert sets `extract_state = 'pending'` when a file is new or its size or whole-second mtime changed (the same comparison `updated_at` already uses). A document whose `extractor_version` is older than its format's current reader version is re-extracted on the next pass, staying linked and searchable until the new text replaces it (as built in Slice 1.6, ADR 0011). Nothing else triggers re-extraction.
- **Transactions.** Each document commits in **one** transaction: `documents` row, all `passages`, the FTS trigger inserts, and the instance link. A kill at any point leaves either the whole document or nothing (P1-12). The cap of 20 million characters per document bounds the transaction.
- **FTS maintenance.** `INSERT INTO passages_fts(passages_fts) VALUES ('merge', N)` in bounded batches at the end of each extraction job; `'integrity-check'` in tests; `'rebuild'` as the recovery action if a check fails.
- **Orphans.** A document is deleted only when its last instance is purged (an instance is purged when its source root is removed or when the user deletes a missing file from the catalog; Phase 1 does not auto-purge).
- **Readers.** `CatalogReader::POOL_SIZE` rises from 2 to 4: a 200 ms search must not starve the status poll, the Windows regression Slice 0.8 fixed.
- **Migration.** Opening a Phase 0 (schema-6) catalog applies 0007 onward. A `paused_user` scan and its frontier survive the rename intact (P1-01). Newer schemas are still refused without writes.
- **Migration rules** (also in `src-tauri/migrations/README.md`). The runner applies each migration in a transaction on a connection with foreign keys **on**, so: never add a `REFERENCES` clause to a table that doesn't exist yet (every later insert into the child fails with "no such table"); and never rebuild a table that other tables reference (dropping it cascades). That is why `jobs.kind` has no CHECK constraint: a new kind would need a rebuild. Rust's `JobKind` refuses unknown values when decoding.

## 7. Jobs, extraction worker and policy

### 7.1 Jobs

- One `jobs` table, the Phase 0 state machine (§7.3 of the Phase 0 spec) and `transition()` unchanged. Two runner threads, `JobRunner<ScanProcessor>` and `JobRunner<ExtractProcessor>`, each selecting `next_runnable` filtered by its kind, so a scan and an extraction can run at the same time through the one writer. `pause_job`, `resume_job` and `cancel_job` take any job id.
- An **extract job is per source root**, like a scan, so it reuses `active_for_root`, `priority` and the `source_root_missing` pause. It is enqueued when a scan of that root finishes with pending files, by `recover_on_open` for roots with pending files, and by **Read text now**. Enqueueing is idempotent: one active extract job per root.
- Its queue is `file_instances WHERE source_root_id = ? AND availability = 'available' AND extract_state = 'pending'`, read in small batches ordered by `rowid`. There is **no frontier**: each file's result is one transaction, so pause, cancel, kill and resume need no checkpoint beyond the rows.
- Progress is a live count per root over the covering index (the migration-0006 pattern): pending, done, and each blocked state. The `extraction-status` event carries an `ExtractionOverview` at most four times a second plus on every state change, like `inventory-status`.
- Cataloging keeps running while extraction runs; neither waits for the other. A scan that marks a file missing does not touch its document.

### 7.2 Extraction worker

- **Process model.** The Atlas executable re-run with the hidden argument `--extract-worker`, checked in `main.rs` before `atlas_lib::run()`, so no window, plugin or workspace is initialised. The supervisor in the main process owns the child: it spawns it with piped stdio, speaks the protocol, kills it, and restarts it. PDFium never runs in the main process.
- **Protocol.** JSON lines, one request at a time. Worker → `hello {protocol: 1, extractorVersion, pdfiumVersion}` on start. Requests: `hash {path}` → `hashed {hash, size, mtimeSecs}`; `extract {path, kind, maxChars}` → zero or more `page {ordinal, pageNo?, location, text}` frames, then `done {pageCount, charCount, title?, authors?, language?}` or `failed {outcome, reason}` where `outcome` is one of `encrypted | malformed | unsupported | too_large | needs_ocr`. A panic inside the worker is caught and reported as `failed {outcome: 'malformed', reason: 'panic: …'}` (no paths in reasons).
- **Protocol hygiene.** PDFium or libc may print to file descriptor 1. The worker duplicates fd 1 to a private descriptor for the protocol and points fd 1 at stderr (`SetStdHandle` on Windows) before loading PDFium.
- **Reading files.** The worker opens the file itself (on Windows with `FILE_FLAG_OPEN_NO_RECALL`, so an offloaded file is never recalled by accident), **sniffs the signature** (`%PDF-` within the first 1,024 bytes; a zip whose first entry is `mimetype` = `application/epub+zip`), hashes the bytes streaming, then seeks to the start and parses with `load_pdf_from_reader` or `rbook`. A signature that contradicts the extension is `unsupported` with reason `not_a_pdf` / `not_an_epub`.
- **PDF.** One passage per page via `PdfPageText::all()`. `PasswordError` and `SecurityError` → `encrypted`; other PDFium errors → `malformed`. Metadata from the Info dictionary (Title, Author, Lang).
- **EPUB.** Pre-scan with `zip`: `mimetype` first; `META-INF/encryption.xml` present → `encrypted` **before any chapter is read**; an entry name that escapes the archive → `unsupported` (`zip_slip`); any entry over 32 MiB uncompressed or the archive over 256 MiB → `too_large`. Then `rbook` in spine order; each spine document converted with `html2text` (plain, no decoration) and split at paragraph boundaries into pieces of at most 4,000 characters; `location` is "Chapter *n*, part *m*"; `page_no` is NULL. Metadata from `dc:title`, `dc:creator`, `dc:language`.
- **Unreadable text** (ADR 0008's rule): after extraction, if at least half the pages have fewer than 20 visible characters, or at least 10% of all characters are control, private-use or U+FFFD characters, the outcome is `needs_ocr` and no passages are stored. EPUBs are exempt from the half-pages rule.
- **Caps.** `maxChars` is 20,000,000 per document; past it the worker stops with `too_large`. Per-page text over 1,000,000 characters is truncated with a reason recorded.
- **Supervision.** One worker by default (`ATLAS_EXTRACT_WORKERS`, debug builds, up to 2). A 30-second **inactivity** timeout per request (a frame resets it), so a 3,000-page book is fine and a hung parser is not. RSS cap `min(1 GiB, 15% of total memory)`, polled with `sysinfo` every 250 ms. On timeout or cap: kill (SIGKILL / `TerminateProcess`), mark the file's attempt, restart the worker. Two crashes on the same file → document `worker_crashed`; ten worker restarts within one job → job `failed_retryable` with backoff. Quitting the app kills the worker before the catalog closes (P1-22).
- **Startup cost.** Measured in Slice 1.2. If the GUI executable in worker mode takes more than about 300 ms to reach `hello` on any OS, Phase 1 falls back to a separate `[[bin]] atlas-extract` sidecar; the protocol does not change.
- **Locating PDFium.** macOS: `Contents/Frameworks/libpdfium.dylib` next to the executable; Windows and Linux: the Tauri resource directory. Debug builds honour `ATLAS_PDFIUM_LIBRARY` (as the only candidate) and otherwise fall back to the fetched `src-tauri/vendor/pdfium/<platform>/`, so `cargo test` needs no setup beyond `just setup`; release builds ignore the variable. A missing or unloadable library fails the job with `extractor_unavailable` and a visible message; it never crashes the app.

### 7.3 Policy

- Extraction follows the Phase 0 policy. **Paused** (battery ≤ 20% with the 25%/AC hysteresis, low space, data root missing, source root missing, memory or thermal pressure) stops the job in `paused_policy` with the reason. **Slow** (on battery, power unknown, system busy) and **Run** set a **duty cycle** rather than operations per second, because files vary by orders of magnitude: after each file the worker loop sleeps `elapsed × (1/duty − 1)`, with duty 50% when running and 20% when slowed. `ATLAS_EXTRACT_UNTHROTTLED` (debug builds) removes the sleep.
- Policy is re-checked between files and at least every 2 seconds, as for scans. A paused extraction resumes from the next pending file; nothing is re-done.
- Cloud-only files (`storage = 'cloud_only'`, or dataless at open time) are `skipped_cloud_only` with no read; they are re-checked on each pass and extracted once they are local. The count is shown ("312 files are only in the cloud").
- Scans and extraction share the one writer; there is no priority between them beyond the policy. Interactive work (search, browse, open) is not blocked by either.

### 7.4 Search

- **Query builder.** User input is split into terms and quoted phrases; every term is quoted for FTS5 (`"term"`), so `*`, `NEAR`, parentheses, `AND`/`OR`/`NOT` and quotes in the input are never operators. Only a trailing `*` on a term becomes a prefix query. Terms are implicitly ANDed. Empty or punctuation-only input returns an empty page, not an error.
- **Ranking.** `bm25(passages_fts)`; `ORDER BY rank LIMIT 500` first, then group by document in Rust and keep the best passage per document, so a common term over a million passages never groups every match. Name matches from Phase 0's `search_key LIKE` are merged with `why: name`; a document matching both ranks above text-only matches of equal rank.
- **Snippets.** FTS5 `snippet()` with private-use markers, converted in Rust to `SnippetSpan { text, hit }` lists. Snippet length about 24 tokens.
- **Filters and paging.** Folder and type filters apply in SQL; `includeMissing` as in the Library. Paging is over the grouped list; the page reports how many documents matched and whether extraction is still running.
- **Limitation (documented in the UI help and ADR 0010).** `unicode61` treats a run of CJK characters as one token, so Chinese or Japanese text is found only by whole runs. A trigram index is the Phase 4 option; the evaluation corpus is written in Latin-script languages for this reason, and 道德經 is included only to prove it extracts without error.

## 8. Cross-platform system probes and minimum hardware

Unchanged from Phase 0. Phase 1 adds:

- `StorageUsage { catalogBytes, textBytes, indexBytes, otherCatalogBytes, cacheBytes, dataVolumeFreeBytes, estimate, measuredAt }`, computed from file sizes and, as built in Slice 1.6, SQLite's `dbstat` (exact; it is compiled into the bundled SQLite). The original plan was `dbstat`-free queries approximating the FTS and passage sizes.
- The **indexing estimate**: once at least 50 documents are done, extrapolate from the mean characters per done file to the pending files, with a range of ±40% and a DB growth factor measured by the benchmarks (§13). Never a point value; never shown before 50 samples.
- Memory and thermal pressure now also pause extraction (they already pause scans).

## 9. Service interfaces and extension-ready design

```rust
// Phase 0 traits stand. Phase 1 promotes and fills in:
trait JobProcessor: Send { fn idle_wait(&self) -> Duration; fn started(&mut self) {} fn run(&mut self, job: JobRow) -> Flow; }
// Built per open root by a factory from ProcessorContext { catalog, data_root, signal }; one runner per JobKind.
// SchedulingPolicy stays a concrete ResourcePolicy: there is still one implementation.
trait DocumentProcessor { fn id(&self) -> &'static str; fn supports(&self, FileType) -> bool;
                          fn extract(&mut self, req: ExtractRequest, sink: &mut dyn PageSink) -> ExtractOutcome; }
// The in-process façade over the worker implements DocumentProcessor; tests inject a scripted one.
```

**Abstraction budget:** `JobProcessor` earns its trait because two kinds exist and tests drive a fake processor through `JobRunner`. `DocumentProcessor` earns its trait because the extraction tests inject a scripted worker (P1-11). `SchedulingPolicy` stays concrete. The name `DocumentProcessor` is kept from Phase 0; the product spec calls it `DocumentExtractor`, and this document is the record that they are the same thing.

New typed IPC commands (each registered in the slice that implements it; `tests/ipc_contract.rs` forbids stubs):

```text
start_extraction(source_root_id) -> job_id          # idempotent; "Read text now"
get_extraction_status() -> ExtractionOverview        # also pushed as the extraction-status event
search_documents(query: SearchQuery) -> SearchPage
get_document_detail(file_instance_id) -> DocumentDetail  # file, title, authors, language, pages, status, Also at
list_duplicates(query: DuplicateQuery) -> DuplicatePage   # Slice 1.6
get_storage_usage() -> StorageUsage
clean_caches() -> CleanCachesResult                      # { freedBytes, usage }
```

DTOs: `JobKind`, `ExtractionState` (the effective per-file status), `ExtractionOverview`, `SearchQuery { text, sourceRootId?, fileType?, includeMissing, page, pageSize }`, `SearchHit`, `SnippetSpan`, `SearchPage`, `DocumentDetail`, `StorageUsage`. `FileInstance` gains `documentId` and `extraction { state, reason? }`. All are ts-rs exported; `check-bindings` covers them.

Events: `inventory-status` unchanged; `extraction-status` added, throttled the same way.

**Security boundary.** The renderer's capabilities stay `core:default`. The worker receives one file path per request and nothing else: no database, no keys, no network, stdio only, killed on any misbehaviour. Errors and events never carry paths or extracted text. Snippets are the only document text that crosses IPC, and they cross as spans. Search input is data, never FTS syntax. Source files remain read-only (the worker opens them read-only and never writes beside them).

## 10. Evaluation corpus and search quality

- `fixtures/evaluation/questions.json`: about 40 entries, `{ "id": "q01", "question": "…", "relevant": ["epub-pg11"], "kind": "exact" | "phrase" | "paraphrase" }`, written by hand against the 33 spike-corpus documents (Alice, The Raven, Candide, The Prince, six CC BY arXiv papers, two Federal Register documents, the generated PDFs). About a third quote the text exactly, a third use a phrase in quotes, a third paraphrase in the user's own words. Document ids are those of `spikes/extraction-bench/corpus/manifest.json`.
- `tests/search_eval.rs` builds a library from the corpus, runs extraction, asks every question through the search service, and asserts **hit@5 ≥ 0.80**. It prints the per-question ranks so a regression names the question. This is a real test, not a benchmark (P1-17).
- **Personal questions.** The same JSON format over the user's own library: `just eval <questions.json> [data-root]` runs `benches/search.rs --eval`, which opens a catalog **read-only** (reader connections only, no lock, no migrations) so it can run against the real library while the app is closed. Results are printed, never committed; the format is documented in `fixtures/README.md`.

## 11. Test fixture collection

Phase 0's generator (`scripts/generate-fixtures.mjs`) and its layout stand, as §11 of the Phase 0 spec required. Phase 1 adds **body text**:

- `--text-kb N` (default 0 keeps the Phase 0 output byte for byte). Each file gets deterministic pseudo-text from a seeded word list, about N KB, with a unique marker phrase per file (`"<subject> marker <n>"`) and a few shared topic phrases, so questions over the fixture are derivable and repeatable.
- PDFs become multi-page documents (about 3,000 characters a page, Helvetica, WinAnsi `Tj` strings, a correct cross-reference table), so pages and page numbers mean something.
- EPUBs become **valid** zips: `mimetype` first and stored, `META-INF/container.xml`, an OPF with `dc:title`, `dc:creator` and `dc:language`, and XHTML chapters (as built: four chapters, entries stored rather than deflated so the bytes are the same on every machine; see `fixtures/README.md`).
- 30,000 files × 100 KB ≈ 3 GB locally (measured: 3.6 GiB in 49 s); CI uses `--text-kb 20` (measured: 808 MiB in 12 s) because of runner disk. The 1,000-file set uses the same options. Note: without `--text-kb`, every PDF is byte-identical and the whole fixture collapses to one document by hash; the extraction gate is only meaningful with text.
- The spike corpus (`spikes/extraction-bench/corpus/`, 33 documents, 9.4 MiB, every file with source, license and SHA-256) is the extraction test input; `just spike-corpus` keeps verifying it. It covers born-digital, two- and three-column, ligatures and accents, a scanned book, a PDF without ToUnicode maps, owner- and user-password PDFs, DRM, zip bombs and zip-slip, broken cross-reference tables and empty files.
- The rule stands: never commit personal books or papers; fixtures must never mutate a user's collection.

## 12. Required automated tests

| ID | Scenario | Expected result |
| --- | --- | --- |
| P1-01 | Open a schema-6 catalog holding a `paused_user` scan with a frontier | Row is now `jobs` with `kind = 'scan'`; pause kept; frontier intact; Phase 0 behaviour unchanged |
| P1-02 | A scan and an extract job are runnable at the same time | Both run through the one writer; pausing either stops only that job; no writes after a pause |
| P1-03 | Rescan after one file's modification time changes | Only that file is re-hashed and re-extracted; a snapshot of every original's bytes, mtime and hash is unchanged after the pass |
| P1-04 | Spike corpus end to end through the app's job | Every document's effective status matches its manifest `expect` (text, needs-ocr, encrypted, no-crash) |
| P1-05 | 30,000-file fixture with `--text-kb` | 100% of well-formed files `done`; every other file has a status and reason; no silent skips |
| P1-06 | The same bytes in two folders | One document, two instances; extracted once; both locations shown |
| P1-07 | A file moved (missing + new instance) | The new instance links to the existing document by hash; no re-extraction |
| P1-08 | PDF without ToUnicode maps; scanned PDF | `needs_ocr`; zero false positives across the text documents of the corpus |
| P1-09 | DRM EPUB; user-password PDF | `encrypted`; no chapter is read past `encryption.xml`; the owner-password PDF extracts |
| P1-10 | 64 MiB chapter; zip-slip entry; plain text named `.pdf` | `too_large` / `unsupported` with reason; worker RSS stays under the cap |
| P1-11 | Scripted worker hangs, exceeds RSS, or exits mid-document | Killed and restarted; the file is `worker_crashed` after two tries; the job continues with the next file |
| P1-12 | SIGKILL of the app mid-document | No partial document or passages; FTS `integrity-check` passes; relaunch completes the rest without duplicates |
| P1-13 | Cloud-only file in the queue | `skipped_cloud_only`; the file is never opened (recording source / no-recall); counted in status |
| P1-14 | Battery 50% → 20% → 25% during extraction | Slowed, then paused with `battery_low`, then resumed; the same hysteresis as P0-10 |
| P1-15 | Search input containing `"`, `*`, `NEAR(`, parentheses, `AND` | No FTS syntax error; prefix only for a trailing `*`; the words are searched as words |
| P1-16 | Phrase and diacritics (`"café crème"`, `CAFE CREME`) | The phrase is respected; accent- and case-insensitive; the generated `features.pdf` is found |
| P1-17 | The committed evaluation corpus | hit@5 ≥ 0.80; per-question ranks printed |
| P1-18 | Title and authors from PDF Info and EPUB `dc:` metadata | Shown in results and detail; file-name fallback when absent; snippet spans carry no markup |
| P1-19 | Search latency on the 30k text fixture while extraction runs | p95 < 250 ms on the reference machine (asserted in the benchmark run locally, reported on CI) |
| P1-20 | Pause and resume inside the 20,000-entry folder | Under 3 s from resume to the next new row at the policy rate |
| P1-21 | Windows junction inside a source root; a 300-character path | The junction is skipped and counted like a symlink; the long path is cataloged and extracted |
| P1-22 | Quit (and SIGKILL) during extraction | No worker process survives the app; on relaunch the job resumes |

Use the injected fakes from Phase 0 (`FakeMonitor`, `FaultySource`) plus a scripted `DocumentProcessor` for P1-11. Add migration tests for 0007, 0008 and 0009, FTS integrity checks after every crash test, and the originals-unchanged snapshot to every extraction test. Negative cases must be tested, not just the happy path.

## 13. Benchmarks and hardware validation

Record performance, don't invent guarantees. At minimum measure, on the 1,000- and 30,000-file fixtures with body text:

- Extraction throughput (files/s, pages/s), CPU % of one core at the Run duty cycle and unthrottled, worker peak RSS and restarts, and the main process's RSS.
- Catalog growth: database and FTS size per 1,000 files and as a ratio to the text volume; WAL size during a pass; time for the end-of-job FTS merge.
- Time from adding a folder to the first searchable document; time to a fully searchable 30k library.
- Search latency (p50, p95, max) while extraction runs and when idle, for a single term, a phrase, a prefix, a rare term and a common term; cold (first query after reopen) and warm.
- Pause and resume cost; recovery after a forced kill (duplicates must be 0).
- AC versus simulated battery; the duty cycle's effect on wall time.
- macOS arm64 reference machine first; Windows x64 and Linux x64 CI evidence before sign-off.

Provisional targets, to report against and not to publish as requirements: **search p95 under about 250 ms** while extracting on the 30k text fixture; **extraction at or above 200 pages/s** on one core at the Run duty cycle on the reference machine; **catalog growth at or below 1.5×** the extracted text volume. Revisit the duty cycle and the caps from these measurements, not assumptions. `benches/extraction.rs` and `benches/search.rs` follow `benches/inventory.rs` (JSON report, `scripts/bench.mjs` scenarios, Markdown tables in `docs/benchmarks.md`).

## 14. Cross-platform packaging, CI and release evidence

- **PDFium delivery.** `tools.lock.json` at the repository root pins the bblanchon/pdfium-binaries release (chromium/8125) per platform with SHA-256; `scripts/fetch-pdfium.mjs` verifies and unpacks it into gitignored `src-tauri/vendor/pdfium/<platform>/` with its `LICENSE` and `licenses/` folder. `just setup` and `.github/actions/setup` run it (cached by hash) because `tauri-build` copies bundle resources at compile time.
- **Bundling.** macOS: `tauri.macos.conf.json` lists the dylib under `bundle.macOS.frameworks`, so Tauri places it in `Contents/Frameworks` and signs it with the app's identity (ad-hoc in CI), which hardened-runtime library validation requires. Windows and Linux: `tauri.windows.conf.json` / `tauri.linux.conf.json` ship `pdfium.dll` / `libpdfium.so` as `bundle.resources`. The notices ship as `licenses/pdfium/` resources on every OS.
- **Signing gate (Slice 1.2).** The CI-built macOS `.app` passes `codesign --verify --deep --strict` with the dylib inside, and its worker reaches `hello` when launched from the bundle, after extracting the two-page self-test PDF built into the binary (as built; see the Slice 1.2 revision note). If the ad-hoc build fails library validation, add `com.apple.security.cs.disable-library-validation` through `bundle.macOS.entitlements`, and record that notarization will later need the same entitlement reviewed.
- **CI.** The full tier (docs/architecture.md, "Build, CI and measurement") carries everything below; the quick tier for slice branches runs only the Linux checks and tests. The smoke test gains "search finds a passage" (after extraction finishes on the 1k text fixture) and "quit leaves no worker process". The bench job adds the 30k text scenario (`--text-kb 20`). The spike workflow is unchanged.
- **Licenses.** `docs/dependency-licenses.md` gains `pdfium-render`, `rbook`, `html2text`, `zip`, `blake3` and the hand-kept native row for PDFium (BSD-3-Clause plus FreeType, libjpeg-turbo, OpenJPEG, Little CMS, zlib, Abseil, HarfBuzz, ICU and the rest of its `licenses/` folder). GPL/AGPL findings are flagged as before.
- **Linux dependencies** are unchanged: PDFium is bundled, not a system package. Document the AppImage's bundled PDFium with the other bundled libraries.
- The artifacts still start without Python, Docker, a database server, Ollama or an LLM, and extraction needs none of them either.
- Smoke on all three OSes covers: catalog, extraction to completion, a text search that finds a passage, open from a result, pause and resume of extraction, quit and relaunch. Where automation can't reach (native dialogs, the OS opening a file), the Phase 0 manual checklist is extended and executed on macOS.

## 15. Implementation backlog: ordered vertical slices

Implement and verify **one slice at a time**. Build and rerun tests after each slice, then proceed. Never claim later slices are done because scaffolding compiles.

| Slice | Deliverable | Verification gate |
| --- | --- | --- |
| 1.1 | **Jobs.** Migration 0007 (jobs only); `services/jobs` (`JobRunner`, `JobProcessor`, generic control); inventory re-homed as `ScanProcessor`; pause/resume/cancel for any job id; "Phase 1 slice 1.1" in the app status, status bar, error message and mocks; ADR 0009 drafted | Every Phase 0 test green without semantic change; P1-01, P1-02 |
| 1.2 | **Foundations.** Fixtures with body text and valid EPUBs; `tools.lock.json` and `fetch-pdfium`; PDFium bundled and signed on three OSes; worker skeleton that loads PDFium and answers `hello` | Bundles build on three OSes; `codesign --verify --deep --strict`; `hello` from the CI-built `.app` on macOS and from `cargo test` everywhere; worker start time recorded (sidecar fallback decided) |
| 1.3 | **Extraction.** Migration 0008 (documents and the `file_instances` extraction columns); engines (sniff, hash, PDF, EPUB, unreadable-text rule), documents and dedup, per-file status, supervisor, duty-cycle policy, `extraction-status`, Library Text badges and filter, status-bar line, Read text now; ADR 0009 final | P1-03 … P1-14 |
| 1.4 | **Search.** Migration 0009 (passages and FTS5 with triggers), query builder, `search_documents`, the evaluation questions and `search_eval`, `benches/search.rs`; ADR 0010 | P1-15 … P1-17, P1-19 |
| 1.5 | **Search UI and document detail.** Home → Search view (a main tab), results with spans, filters, paging, the full detail panel (title, authors, language, pages, text status and reason, Also at), job controls for both kinds | Vitest coverage of every new screen state; smoke "search finds a passage" |
| 1.6 | **Metadata quality and duplicates.** Titles from metadata, then a PDF's first page, then the file name; junk metadata cleaned; EPUB text without Markdown; per-format reader versions with a stay-searchable re-read; Library › Duplicates; storage usage by category, indexing estimate, Clean caches | P1-18; estimate shown as a range only after 50 samples |
| 1.7 | **Carry-overs.** Replayed entries charged less on resume inside large folders; Windows junctions and long paths; worker lifetime on quit; `benches/extraction.rs` | P1-20 … P1-22 |
| 1.8 | **Evidence and close-out.** Benchmarks on three OSes in `docs/benchmarks.md`; license report; platform validation including a manual smoke of the release DMG with PDFium on macOS; ADRs final; `docs/phase1-closeout.md`; version 0.2.0 | All CI green on `main`; close-out review against §1, §12, §13, §14 and §16 |

**Ordering notes.** Slices 1.3–1.6 need the fixtures from 1.2; 1.4 needs text from 1.3; 1.7 can run in parallel with 1.5 and 1.6. Nothing in Phase 1 needs the `spikes/` workspaces to build.

Each slice includes: code, automated tests, a short README and docs update (`architecture.md`, the ADRs, `benchmarks.md`, `platform-validation.md`, `packaging.md` and the justfile as they are affected), explicit demo steps, and a brief note of any deviations from this specification. Checkpoints use the Phase 0 convention: a `slice-1N` branch, squash-merged onto `main` as `phase1/slice-0N: …` with an annotated tag `phase1/slice-0N`.

## 16. Acceptance checklist for the product owner

- [ ] Finds a book or paper by a phrase from its text, not just its name.
- [ ] Shows for every file whether its text is ready, waiting, needs OCR, is encrypted, is only in the cloud, or couldn't be read, with a reason in plain words.
- [ ] Never writes to, moves or downloads an original while reading it.
- [ ] Reading text slows on battery, pauses at low battery, low space or quit, and resumes where it stopped.
- [ ] The same content stored in several places appears once, with all its locations.
- [ ] Search and browsing stay responsive while text is being read.
- [ ] Installers on macOS, Windows and Linux include the PDF library and start without Python or any other installation.
- [ ] Rescanning an unchanged library reads no text again.
- [ ] Storage use is shown by category, with an honest estimate of what the remaining text will need.
- [ ] Recorded test, benchmark, license and platform-smoke evidence for the three OSes.
- [ ] Clearly distinguishes what works now from what is still to come (OCR, semantic search, AI).

## 17. Constraints for the coding agent

1. **Read this entire spec, the Phase 0 spec, the Phase 0 close-out review, ADR 0008 and Atlas Product Specification v0.2 first.** The parent spec is product direction; this document is the Phase 1 scope and acceptance contract.
2. Work incrementally, beginning with Slice 1.1. Deliver working code rather than architecture-only commentary. Prefer maintained existing libraries over custom replacements.
3. If implementation conflicts with a stated invariant (read-only originals, single data root, recoverable jobs, no mandatory AI, no Python in the installer), preserve the invariant and document the tradeoff.
4. Do not silently substitute a Python backend, Electron, a standalone server or daemon, an arbitrary runtime plugin system, a cloud account, or an externally managed database.
5. Don't overbuild future phases. Define interfaces and reserve layout where necessary, but implement only what Phase 1 exercises.
6. Validate inputs on the Rust side. Treat file contents, file names, archive entry names and search input as untrusted data. Never run PDFium in the main process; never execute file paths in a shell.
7. Use explicit migrations and structured error codes. Prefer deterministic fixtures, fakes and a scripted worker to flaky real-hardware tests, supplemented by physical-platform smoke tests.
8. Never claim a file has text when the worker reported otherwise, and never show a document as read before its transaction committed. Snippets are spans, not markup.
9. Run formatting, static checks and tests before claiming each slice is done. Report commands executed and outcomes. Never claim a Windows, Linux or macOS build was tested unless it actually ran there; label unverifiable steps.
10. When a measurement contradicts a document (as the spike's `thread_safe` setting contradicted ADR 0008), correct the document in the same change.
11. End each slice report with: implemented, tests passed, manual verification, known issues, and next slice.

## 18. Kickoff instructions for the coding agent

**First requested task:** Implement **Slice 1.1** only: add migration 0007 (rename `scan_sessions` to `jobs` with a `kind` column; the `file_instances` extraction columns wait for 0008 in Slice 1.3, see §6.1), create `services/jobs` with `JobRunner` and the `JobProcessor` trait, move the Phase 0 inventory worker body into `ScanProcessor` without changing its behaviour, make `pause_job`, `resume_job` and `cancel_job` work on any job id, change the slice strings to Phase 1 (`CURRENT_SLICE = "1.1"`, the status bar, the `NotImplemented` message, the test mocks and `ipc_contract.rs`), and draft ADR 0009. Every Phase 0 test must pass unchanged in meaning; add P1-01 and P1-02. Update `docs/architecture.md`, `migrations/README.md`, the README status and the justfile as affected. After the slice passes, summarize code and verification, note deviations, and name Slice 1.2 as the next step.

Do **not** start extraction, FTS, or the search UI before Slice 1.1's gate passes. Do not add the extractor crates to `src-tauri/Cargo.toml` before Slice 1.2.

---

### Revision notes

**2026-11-08, v0.1** (written after the Phase 0 close-out review):

- Product-owner decisions recorded: search target top 5 for ≥ 80%; extraction gate 100% of well-formed files with every other file reported; extraction slowed on battery and paused at ≤ 20% like cataloging; thumbnails, filesystem watchers, an OCR engine and Move Atlas Data deferred.
- Extraction jobs are per source root (reusing Phase 0's root-level pausing and priority) rather than one catalog-wide pass; the jobs migration is a rename plus a `kind` column.
- Content state lives on documents keyed by hash, not on file instances, so duplicates are extracted once and moves keep their text.
- FTS5 is trigger-maintained; text stays in SQLite; `content/extracted/` remains reserved.
- `pdfium-render`'s `thread_safe` feature stays off (the spike measured it that way); ADR 0008's sentence about it is corrected in the same change as this document.
- Fixtures with body text come before any extraction slice, because nothing can be extracted from the Phase 0 stubs.

**2026-11-08, amended during Slice 1.1:**

- Migration 0007 is jobs only. Adding `file_instances.document_id REFERENCES documents(id)` before `documents` exists makes every later insert into `file_instances` fail with foreign keys on ("no such table: main.documents"), so the extraction columns move to `0008_documents.sql` (Slice 1.3) and passages with FTS5 to `0009_passages_fts.sql` (Slice 1.4).
- `jobs.kind` has no CHECK constraint, so a future kind needs no table rebuild (which would cascade-delete the scan frontier under the migration runner's foreign keys).
- The `JobProcessor` signature in §9 is the as-built one.
- CI has tiers (added at the product owner's request). Pushes to `slice-*` branches run a quick Linux-only tier (checks and tests), and docs-only branch pushes run lint only. `main`, tags, pull requests and `just ci-remote` run the full matrix that §14 extends. A slice is still merged only after a full run passes.

**2026-11-08, amended during Slice 1.2:**

- The signing gate's "extracts one PDF" is met by a self-test: before `hello`, the worker extracts a two-page PDF compiled into the binary and checks its marker, and `hello` carries the result (`selfTest {pages, chars}`). Product-owner decision; the `extract` request still arrives in Slice 1.3.
- Ad-hoc signing with the hardened runtime failed library validation for the bundled PDFium ("different Team IDs"), so the planned fallback entitlement `com.apple.security.cs.disable-library-validation` is in `src-tauri/Entitlements.plist`. Notarization must review it.
- The worker stays a mode of the app executable: it reaches `hello` in about 15–20 ms on the M5 Pro and on the CI runners (`docs/benchmarks.md`), far below the 300 ms threshold for a sidecar.
- The spike's 3 ms worker start never loaded PDFium; corrected in ADR 0008 and `spike-0.9.md`.
- PDFium's licence is the archive's `licenses/pdfium.txt`; its root `LICENSE` is the MIT licence of bblanchon's build scripts. Both ship in `licenses/pdfium/` with the other notices.
- `ATLAS_PDFIUM_LIBRARY` is honoured only in debug builds, like the other `ATLAS_*` overrides.
- Because tauri-build checks the bundled library on every compile, every Rust build needs the fetched PDFium (`just setup`); development is supported on the three pinned platforms only.
- Fixture EPUB entries are stored, not deflated, so fixtures are byte-identical on every machine (Node's zlib output differs between CPUs). The flat folder also gets body text, so the 30k text fixture doesn't collapse to one document.

**2026-11-09, amended during Slice 1.3:**

- `passages` is created by `0008_documents.sql` (Slice 1.3), so the text read in 1.3 is kept; `0009` (Slice 1.4) adds only the FTS5 table and its triggers, built from the stored passages with `'rebuild'`. Product-owner decision.
- `file_instances` also gains `extract_attempts` (two crashes make a file `worker_crashed`). `extract_state` has no `hashing` value: "Reading…" is the file the active job reports.
- The EPUB pre-scan (DRM, zip-slip, size caps) runs before rbook reads anything; `encryption.xml` that only obfuscates fonts is not DRM. Size caps use the declared sizes, so nothing is inflated to check.
- The extraction status line shows "N of M files" only when no scan is finding files; otherwise "N files so far".
- One worker per library; `ATLAS_EXTRACT_WORKERS=2` moves to Slice 1.7. The FTS `integrity-check` part of P1-12, the FTS merge, and the reader pool of 4 move to Slice 1.4 with FTS5.
- Request paths are the catalog's platform-tagged bytes, hex-encoded, so any path survives JSON.

**2026-11-09, amended during Slice 1.4:**

- Search ANDs the words; when fewer documents than one page match, an any-words form (common words dropped) fills the page after them, marked `allWords: false`. Product-owner decision; all 13 paraphrase questions depended on it (27/40 with AND only).
- `0009_passages_fts.sql` adds only the FTS5 table and triggers over the passages 0008 already stores, and rebuilds the index once.
- The FTS `integrity-check` part of P1-12 is checked here (`tests/extraction_crash.rs`). The reader pool is 4.
- The evaluation questions were written and committed before search was built; they all pass at rank 1 on the 17-document corpus, a floor check rather than a quality claim.
- Catalog growth on the 30k text fixture is 2.3× the text (provisional target 1.5×), recorded in ADR 0010 for Slice 1.7.

**2026-11-09, amended during Slice 1.5:**

- The full document detail panel ships in 1.5 (title, authors, language, pages, size, modified, folder and path, text status with its reason, Also at with Open and Reveal per copy), since all of it exists after 1.3. Slice 1.6 narrows to metadata quality (P1-18 fallbacks), a duplicates view and Settings › Storage. Product-owner decision.
- Search is a main tab (Home, Search, Library, Settings; Cmd/Ctrl+1–4). Home's box submits to it; nothing is searched while typing. Product-owner decision.
- `get_document_detail(file_instance_id)` replaces `get_document(document_id)`: every Library row and result has a file ID, while a file not read yet has no document.
- The smoke test generates the 1,000-file fixture with `--text-kb 4` and reads it fully; it also checks that no `--extract-worker` process outlives quitting while text is being read (the observable half of the Slice 1.7 carry-over).
- Typographic double quotes (`“ ” „ ‟ « »`, full-width `＂`) quote phrases like `"`, because macOS smart quotes type them (found in the manual check).
- Search results carry no Text badge (`SearchHit` has no text status, and a text match is read by definition); the detail panel shows it.

**2026-11-09, amended during Slice 1.6:**

- Titles come from the file's metadata when usable, then (PDFs) the largest text on the first page, then the tidied file name; the source is shown ("Title from the first page", "Title from the file name"). Product-owner decision: the product owner asked whether a title could come from the contents; generated titles need a model and wait for the AI phases. Cleaning (placeholders, "Microsoft Word - …", file-name titles; authors split on `;`, account names dropped) happens when the catalog is read, so new rules never need a re-read. EPUBs use `dc:title` or the file name.
- EPUB text is written without Markdown marks (`TrivialDecorator`), fixing `# Chapter 1` in snippets.
- Reader versions are per format (`READER_VERSION_PDF`, `READER_VERSION_EPUB`, both 2). A document read by an older reader stays linked, searchable and "Text ready" while one copy is read again; documents whose output would not change move to the new version unread (ADR 0011). Product-owner decision: re-read EPUBs and untitled PDFs, staying searchable.
- Duplicates live in the Library behind a Files | Duplicates switch, not in a new tab (product-owner decision). `list_duplicates` is a new command; a group needs two available copies; missing copies never count.
- Storage accounting uses SQLite's `dbstat` (compiled into the bundled SQLite), not the `dbstat`-free approximation §8 asked for: it is exact and fast (about 9 ms on the 1k text fixture). `clean_caches` returns `CleanCachesResult { freedBytes, usage }`. The estimate uses the measured text and index bytes per read document, so no separate growth factor is needed.
- The fixture generator gains `--untitled-every N` (no metadata title; PDFs set it in 24 pt on page 1) and `--copies N` (identical copies for duplicate groups); default output is unchanged byte for byte.

### Primary reference documentation (verify current versions at implementation time)

- SQLite FTS5 (external content tables, triggers, `snippet`, `bm25`, `merge`): <https://www.sqlite.org/fts5.html>
- pdfium-render: <https://docs.rs/pdfium-render/latest/pdfium_render/>
- PDFium prebuilt binaries: <https://github.com/bblanchon/pdfium-binaries/releases>
- rbook: <https://docs.rs/rbook/latest/rbook/>
- html2text: <https://docs.rs/html2text/latest/html2text/>
- BLAKE3: <https://docs.rs/blake3/latest/blake3/>
- Tauri v2 bundle configuration (`resources`, `macOS.frameworks`, `entitlements`): <https://v2.tauri.app/reference/config/#bundleconfig>
- Tauri v2 macOS signing: <https://v2.tauri.app/distribute/sign/macos/>
- Apple hardened runtime and library validation: <https://developer.apple.com/documentation/security/hardened-runtime>
