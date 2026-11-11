# Atlas — Phase 3 Implementation Specification v0.1

**Status:** Ready for coding-agent implementation  
**Date:** 2026-11-10 (see Revision notes at the end)  
**Parent:** [Atlas Product Specification v0.2](product-spec.md) §3, §5, §8, §10, §13; builds on the [Phase 2 Implementation Specification](phase2-spec.md), the [Phase 2 close-out review](phase2-closeout.md), [ADR 0001](adr/0001-local-ipc-no-server-single-data-root.md) (the single data root), [ADR 0009](adr/0009-jobs-and-extraction.md) (the extraction worker), [ADR 0011](adr/0011-re-reading-without-losing-search.md) (re-reading) and [ADR 0015](adr/0015-ask-atlas-answers-citations-injection.md) (answers)  
**Target platforms:** macOS arm64, Windows x64, Linux x64  
**Product:** Atlas Community — local-first personal knowledge discovery  
**Milestone:** The *knowledge workspace*: Atlas remembers what you explored, keeps what you found, and lets you pick it up again

> **Product promise:** Collect freely. Organize minimally. Retrieve intelligently.  
> **Experience rule:** Simple by default; powerful by choice. No AI knowledge or terminal required for ordinary users.

## 1. Mission and definition of done

Phases 0 to 2 made Atlas a librarian: it catalogs a lifelong collection, reads it, finds books and papers by words and by meaning, shows what is related, and answers questions with citations checked against the passages it sent. But it forgets. The last answer lives on the screen until the next question; nothing the user does or decides is kept.

Phase 3 gives Atlas a memory that belongs to the user. It **records research trails** (searches, questions, answers, sources opened), lets the user **keep** places and findings, **group** them under interests and projects with nothing mandatory, **see the cited page**, and **take it all out** as Markdown or JSON.

Phase 3 is complete **only** when the following holds on each of the three target OSes:

1. **Trails are recorded.** Every submitted search, every question with its answer and sources, and every source opened from them is recorded into a trail, on this computer only. The user can pause recording, delete any step or trail, and pin a trail. Unpinned trails expire after 90 days.
2. **A session can be resumed after weeks.** With files moved, documents re-read by a newer reader and the embedding model switched in between, every step of a trail still shows. A saved answer reads as it did, each citation opens the right document at the right place, and anything that is gone says so. Any search or question can be run again.
3. **Bookmarks and saved findings survive.** They outlive file moves, re-reads and reader upgrades, and are never removed to recover space.
4. **Interests and projects group them**, and nothing requires one. Home shows recent trails, active interests and projects, and suggestions from the library.
5. **A citation or bookmark in a PDF opens the page itself:** rendered inside Atlas, with the passage highlighted when it can be found, in a process that can be killed.
6. **Everything the user created can leave and come back:** Markdown to a folder they choose, JSON for a complete copy, and an import that restores it.
7. **User data is never lost** to Clean caches, a crash, a kill at any point or a damaged catalog, and it is snapshotted without being asked.
8. Unit, integration and smoke tests, benchmarks, packaging and licence evidence are recorded for macOS, Windows and Linux.

**Do not mark Phase 3 complete on builds alone.** Item 2 is measured by a committed resume scenario (§10) on every OS, and by the product owner resuming a real session from the release build. If an OS is unavailable for a check, document it as unverified with a reproducible checklist, as Phases 0 to 2 did.

### What Phase 3 builds on

- **Stable facts** (measured, in the close-outs): 0.950 hybrid hit@5 on the vague set; Ask Atlas answering in about 26 s with a 27B model on the reference machine; every citation checked.
- **Unstable identifiers** (found while planning): `passages.id` and `chunks.id` do not survive a re-read, and `file_instances.id` does not survive a move. Only `documents.id` (kept across re-reads) and the content hash are durable. §6.2 follows from this.
- **Reserved and empty:** `exports/` and `backups/` in the data root; `cache/previews/`.
- **The product-owner decisions behind this document (2026-11-10):**
  - a **PDF page view only**, no EPUB renderer;
  - trails **recorded automatically**, with pause, deletion and a 90-day expiry for unpinned trails;
  - Markdown export to **a folder chosen each time**;
  - three carry-overs from the Phase 2 close-out: the **iCloud re-read fix**, a **per-model weak margin**, and **Project Gutenberg boilerplate removed at extraction**;
  - no `atlas://` links and no fetch of newer recommendations in this phase.

## 2. Scope boundaries

### Implement in Phase 3

- **The knowledge store:** a second, small SQLite database for everything the user creates, with snapshots.
- **Research trails:** automatic recording, sessions, pause, expiry, deletion, notes, and resuming.
- **Bookmarks** (a place in a document) and **saved findings** (an answer or a passage, with its sources and the user's note).
- **Interests and projects:** one grouping with two kinds.
- **Rediscovery:** "From your library" suggestions, from stored vectors, with no model call.
- **The PDF page view:** a rendered page with the passage highlighted, from citations, bookmarks, results and trail steps.
- **Export and import:** Markdown (Obsidian-style or plain links) to a chosen folder; JSON of the whole store; import with a dry run.
- **Home:** recent trails, active interests and projects, suggestions.
- **Settings:** recording and retention in Privacy; "Your trails and notes" and snapshots in Storage.
- **Carry-overs:** the iCloud re-read fix, the per-model weak margin, Project Gutenberg boilerplate at extraction.
- **Benchmarks, CI, packaging and licences** for the above; smoke steps for the resume flow.

### Explicitly defer

An EPUB renderer, text selection and drawn annotations on a page (a later phase's reader); `atlas://` deep links (notes carry stable ids so they can be added without re-exporting by hand); a "Check for newer recommendations" fetch; a reranker, document summaries, hierarchical retrieval and query rewriting; §7.7 of the Phase 2 spec (an Ask not running while the service is marked unreachable); an Anthropic native adapter; an Atlas-managed runtime and model downloads; OCR, backups of the catalog, integrity tools, diagnostics, Move Atlas Data and compressing stored passages (Phase 4); sync of trails and bookmarks (Phase 5); a rich-text or large note editor, graph views, tags and inferred topics (product spec §14); filesystem watchers; multiple windows; sharing.

**Scope discipline.** One executable. Two SQLite databases, each with one writer: the catalog, and the knowledge store. Traits only where a test injects a fake. Nothing the user creates is ever derived, and nothing derived is ever the only copy of something the user created.

## 3. Technology choices (Phase 3 additions)

| Concern | Phase 3 choice | Notes |
| --- | --- | --- |
| User data | a second SQLite file, `library/knowledge.sqlite` (rusqlite, WAL, STRICT tables, 4 KiB pages) | ADR 0016. Its own migrations and writer thread. Small by design |
| Snapshots | `VACUUM INTO` a staged file, `PRAGMA integrity_check`, atomic rename into `backups/knowledge/` | The pattern of `services/catalog/convert.rs` |
| Page rendering | `pdfium-render` 0.9.5 as pinned: `PdfPage::render_with_config`, `PdfBitmap::as_rgba_bytes` | No new crate feature; neither needs `image_api` |
| Text position on a page | `PdfPageText::search` and character bounds | For the highlight; the stored passages have no coordinates |
| PNG encoding | `png` 0.18 as a direct dependency | Already in `Cargo.lock` transitively; MIT OR Apache-2.0 |
| Rendering process | a second instance of the extraction worker's `Supervisor`, for rendering only | ADR 0018. The main process still never loads PDFium |
| Dialogs | `tauri-plugin-dialog`, already registered, called from Rust only | Folder pick for export, file pick for import. No new plugin and no new renderer capability |
| Markdown and JSON | written by Atlas's own code: `serde_json`, and a small Markdown writer | No Markdown library: Atlas only writes it |
| Time | a `Clock` in the knowledge services' dependencies (new): the system clock in the app, a settable one in tests | Today `infrastructure/clock.rs` is a plain function. Sessions, expiry and snapshots depend on elapsed time, and the resume tests move it by weeks |

## 4. Repository layout

Phase 2's layout stands. Phase 3 adds (new files marked *new*):

```text
fixtures/knowledge/resume-scenario.json        # new: the committed resume scenario (§10)
scripts/generate-knowledge.mjs                 # new: a knowledge store of any size, for benchmarks
src-tauri/
  migrations/0014_boilerplate.sql              # new (Slice 3.1): the catalog's one change
  migrations/knowledge/0001_knowledge.sql      # new (Slice 3.2): the knowledge store's schema
  src/services/knowledge/{mod,store,migrations,anchor,snapshot}.rs     # new (3.2)
  src/services/knowledge/{trails,record}.rs    # new (3.3)
  src/services/knowledge/saved.rs              # new (3.5): bookmarks and findings
  src/services/knowledge/{collections,rediscovery}.rs                  # new (3.6)
  src/services/reader/{mod,render,cache}.rs    # new (3.7)
  src/services/extraction/{protocol,worker_main}.rs   # gain the Render request (protocol version 2)
  src/services/export/{mod,markdown,json,import,dest}.rs               # new (3.8)
  src/commands/{trails,saved,collections,reader,export}.rs             # new
  benches/knowledge.rs                         # new: the store at 100,000 steps
  benches/reader.rs                            # new: page rendering
  tests/{knowledge,knowledge_crash,trails,resume,saved,collections,reader,export}.rs   # new
src/
  views/Trails.svelte                          # new: Trails | Saved | Interests and projects
  lib/components/{TrailList,TrailTimeline,SavedList,CollectionList,CollectionDetail,
                  PageView,SaveMenu,RecordingIndicator,ExportDialog,HomeSections}.svelte   # new
  lib/stores/{trails,saved,collections,reader}.svelte.ts               # new
  lib/{trailCopy,savedCopy,readerCopy,exportCopy}.ts                   # new
docs/adr/0016-…0019-….md                       # new: knowledge store; trails and privacy; page view; export
```

## 5. User experience and screen contracts

Voice, tokens and patterns are those of Phases 0 to 2. New wording lives in copy modules, as before.

### 5.1 Trails (a fifth main view)

- **Navigation** gains **Trails** after Library: Home, Search, Library, Trails, Settings (Cmd/Ctrl+1 to 5). The view has three tabs, a radio group as on Search: **Trails | Saved | Interests and projects**.
- **The list.** Trails by last activity, newest first: the title (or, untitled, its first question or search), the dates, the number of steps, and a pin. A filter field searches titles, questions and searches. Paged like the Library.
- **A trail** opens as a timeline, oldest step first:
  - **A search:** its words and filters, how many documents matched then, the top results then (each opens), and **Search again**.
  - **A question:** the question, the answer as recorded (rendered as on the Ask screen: chips, bold and italic, never markup), the notes that applied (weak, references removed), **Sources then** (the cited ones with their excerpts), and **Ask again**.
  - **A source opened:** the document, the place, how it was reached ("from a citation"), and Open.
  - **A note** the user typed.
- **Actions on a trail:** Continue this trail, Rename, Pin, Add to an interest or project, Export, Delete. On a step: Add a note, Save as a finding (a question), Delete this step.
- **Continue this trail** makes it the current trail: new steps join it. **Search again** and **Ask again** go to the Search screen with the words and filters filled in, and run.
- **What is gone says so.** A source whose file is missing, or whose document Atlas no longer knows, shows its saved excerpt with "This file isn't in your library any more" or "Atlas hasn't read this file yet", never an error.
- **Empty state:** "Atlas keeps a trail of what you search for and ask, on this computer only. Your first search starts one."

### 5.2 Recording

- **An indicator** in the status bar: **"Trail: recording"** or **"Trail: paused"**, a button that toggles it. Paused, nothing is recorded and nothing else changes.
- **New trail** (on the Search screen, beside the mode switch) starts one. Otherwise a new trail starts by itself after 30 minutes without a step.
- **The first time** a step is recorded, Home shows one dismissible note: "Atlas now keeps a trail of your searches and questions on this computer, so you can pick them up later. Pause it or delete trails any time."

### 5.3 Saved: bookmarks and findings

- **Bookmark** is a button wherever a place in a document is shown: a search result, the details panel, a cited passage, a source in Sources used, the page view. It saves the document and, when there is one, the page or location and the passage.
- **Save as a finding** is on a finished answer (with its question and cited sources) and on a passage. A finding is text the user decided to keep.
- **Each has an optional note:** plain text, at most 4,000 characters, edited in place.
- **The Saved tab** lists both, newest first, filterable by kind and by text. Each opens its place (the page view for a PDF page, otherwise the details panel with the passage), edits its note, adds to an interest or project, exports, deletes.
- **The details panel** gains **Bookmarks in this document** when there are any.

### 5.4 Interests and projects

- **An interest** is a name and an optional description. **A project** adds an optional goal and can be marked **Finished**. Either can be archived. There are no tags and no hierarchy.
- **Add to…** on a trail, bookmark, finding or document offers the active ones and "New interest…" / "New project…".
- **A collection's page** lists its trails, findings, bookmarks and documents, with Export and, for a project, Mark as finished.
- **Nothing requires one.** Trails, bookmarks and findings are complete without a collection.

### 5.5 The page view

- **Opens from** a citation chip's panel ("Show the page"), a PDF bookmark, a PDF result, and a trail step. EPUBs keep the passage text in the details panel; the button isn't shown for them.
- **Shows** the page as an image, fitted to the width, with the passage outlined when Atlas finds it on the page. Under the image, the page's text as Atlas read it, so it can be read by a screen reader and copied.
- **Controls:** Previous page, Next page, a page number field ("Page 12 of 340"), Fit width / Actual size, Bookmark this page, Open in the default app, Close. Escape closes and returns focus.
- **While rendering:** "Showing page 12…". If it can't: "Atlas couldn't show this page" with Open in the default app. A scanned page shows without a highlight. A file only in the cloud asks before downloading, as Open does.
- It is a way to see a cited page, **not a reader**: no text selection on the image, no search within the document, no continuous scroll.

### 5.6 Home

Under the search field, each section only when it has something to show:

- **Pick up where you left off:** the three most recent trails.
- **Interests and projects:** the active ones, at most six, then "All interests and projects".
- **From your library:** up to three suggestions, each with its reason ("Related to your interest 'Stoicism'") and **Not for this**. Hidden without a meaning index.

Home never becomes a dashboard: no counts, no charts, nothing that needs organising first.

### 5.7 Settings

- **Privacy › Research trails:** a sentence on what is kept and that it stays on this computer; **Record my trails** (on); **Remove unpinned trails after 90 days** (on); **Delete all trails…** (confirmation naming the count; bookmarks and findings are kept).
- **Storage** gains **Your trails and notes** (the knowledge store) and **Snapshots** (count, size, the newest one's date). Neither is touched by Clean caches. The page-image cache is counted under caches.
- **Export and import** (a new small section): **Export…** and **Import from a file…**.

### 5.8 Export and import

- **Export…** (also on a trail, a finding, a collection) asks what (this item, this collection, everything), the format (**Markdown notes** or **One JSON file**), and for Markdown the link style (**Obsidian links** or **Markdown links**). Then the native folder dialog, starting at the last folder used, or Atlas's own `exports/`.
- **The result** says where it wrote and what: "12 notes written to …/Atlas. 2 already existed and were left alone."
- **Import from a file…** picks a JSON export, shows what it would add ("3 trails, 14 findings and 20 bookmarks are new; 5 are already here"), and adds them on **Import**. It never removes anything.

### Visual and accessibility rules

As Phases 0 to 2. The timeline is a list with headings per step. A recorded answer is not a live region. The page image has alternative text ("Page 12 of The Prince"), and the passage outline never carries meaning alone: the passage is also the text beneath. Every Save action confirms in a polite status ("Bookmarked"). Pinned and paused are said in words, not only by icon.

## 6. Data root, lifecycle and persistence

What the user creates is **irreplaceable** (product spec §8). It lives in its own database, is snapshotted, and is never deleted to recover space. Everything else in Phase 3 is derived: page images are a cache, and suggestions are computed.

### 6.1 The knowledge store

`library/knowledge.sqlite`, created when a library is first opened by Phase 3, with `PRAGMA user_version` as its schema version and its own migration runner (`migrations/knowledge/`). One writer thread (`KnowledgeHandle::call`, the catalog writer's pattern) and a read-only pool. It is opened after the catalog and closed before it.

It references the catalog **by value, never by foreign key**: the two files must each stay valid without the other.

**Migration knowledge/0001 (Slice 3.2):**

```sql
CREATE TABLE trails (
    id           TEXT PRIMARY KEY NOT NULL,        -- uuid
    title        TEXT,                             -- NULL: shown as its first question or search
    pinned       INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    search_text  TEXT NOT NULL DEFAULT '',         -- folded title, questions and searches, for the filter
    step_count   INTEGER NOT NULL DEFAULT 0,
    started_at   TEXT NOT NULL,
    last_step_at TEXT NOT NULL
) STRICT;
CREATE INDEX trails_by_activity ON trails (last_step_at DESC);

CREATE TABLE trail_steps (
    id        TEXT PRIMARY KEY NOT NULL,           -- uuid
    trail_id  TEXT NOT NULL REFERENCES trails (id) ON DELETE CASCADE,
    seq       INTEGER NOT NULL,
    kind      TEXT NOT NULL CHECK (kind IN ('search', 'ask', 'open', 'note')),
    at        TEXT NOT NULL,
    payload   TEXT NOT NULL,                       -- JSON, typed and versioned in Rust (§7.1)
    note      TEXT,
    UNIQUE (trail_id, seq)
) STRICT;

-- Which documents a step touched: for "already opened" and a document's trails.
CREATE TABLE step_documents (
    step_id      TEXT NOT NULL REFERENCES trail_steps (id) ON DELETE CASCADE,
    document_id  TEXT NOT NULL,
    content_hash BLOB NOT NULL,
    PRIMARY KEY (step_id, document_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX step_documents_by_document ON step_documents (document_id);

CREATE TABLE bookmarks (
    id           TEXT PRIMARY KEY NOT NULL,
    anchor       TEXT NOT NULL,                    -- JSON Anchor (§6.2)
    document_id  TEXT NOT NULL,                    -- copied from the anchor, for lookups
    content_hash BLOB NOT NULL,
    note         TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
) STRICT;
CREATE INDEX bookmarks_by_document ON bookmarks (document_id);

CREATE TABLE findings (
    id           TEXT PRIMARY KEY NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN ('answer', 'passage')),
    question     TEXT,                             -- an answer's question
    text         TEXT NOT NULL,                    -- the answer with its [n], or the passage
    sources      TEXT NOT NULL,                    -- JSON: [{ n, anchor }]
    note         TEXT,
    from_step_id TEXT,                             -- not a foreign key: the step may expire
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
) STRICT;

CREATE TABLE collections (
    id          TEXT PRIMARY KEY NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('interest', 'project')),
    name        TEXT NOT NULL,
    description TEXT,
    goal        TEXT,                              -- projects only
    status      TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'finished', 'archived')),
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
) STRICT;

CREATE TABLE collection_items (
    collection_id TEXT NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    item_kind     TEXT NOT NULL CHECK (item_kind IN ('trail', 'bookmark', 'finding', 'document')),
    item_id       TEXT NOT NULL,                   -- a trail, bookmark or finding id; or a document id
    content_hash  BLOB,                            -- documents only
    added_at      TEXT NOT NULL,
    PRIMARY KEY (collection_id, item_kind, item_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE dismissed_suggestions (
    collection_id TEXT NOT NULL,                   -- '' for a suggestion from recent trails
    document_id   TEXT NOT NULL,
    content_hash  BLOB NOT NULL,
    at            TEXT NOT NULL,
    PRIMARY KEY (collection_id, document_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE meta (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL) STRICT;
-- current_trail, current_trail_until, last_snapshot_at, last_snapshot_change, first_step_noted
```

Deleting a trail, bookmark or finding removes its `collection_items` rows in the same transaction. A finding keeps its own copy of its text and sources, so it outlives the trail it came from.

### 6.2 Anchors

A bookmark, a finding's source and a trail step point at a place by an **anchor**, never by a row id:

```text
Anchor { documentId, contentHash, title, fileType,      // the document, and a title to show if it is gone
         pageNo?, location?, ordinal?,                  // the place as it was: "Page 12", "Chapter 3, part 2", the passage's ordinal
         excerpt? }                                     // at most 2,000 characters; absent for a whole-document bookmark
```

An anchor is **resolved when it is shown**, against the catalog as it is now (`knowledge/anchor.rs`):

| State | Meaning | What the user gets |
| --- | --- | --- |
| `found` | The document is known (by id, else by content hash) and the excerpt is at the saved place | Open at the place; the page view for a PDF page |
| `found_elsewhere` | The same content, but the excerpt is now in another passage or on another page (a reader upgrade re-split the text) | Open at the new place |
| `no_file` | The document is known, but no file in the library has that content now (moved away, changed, folder removed) | The saved excerpt, and "This file isn't in your library any more" |
| `unknown` | The catalog has no such document (a rebuilt catalog that hasn't read it yet) | The saved excerpt, and "Atlas hasn't read this file yet"; resolved by content hash once it has |

- **The excerpt is searched for** first at the saved ordinal, then in the document's other passages (folded, whitespace collapsed), then the saved page is used as it is.
- **The file to open** is an available, local file of that document if there is one; a file only in the cloud next; otherwise none.
- **Resolution never writes.** An anchor is not "repaired": the saved place and excerpt are the record of what the user saw.

### 6.3 Snapshots

- **When:** on opening a library and once a day while it stays open, if the store changed since the last snapshot.
- **How:** `VACUUM INTO` a file in `backups/knowledge/staging/`, `PRAGMA integrity_check` on it, then an atomic rename to `backups/knowledge/knowledge-<UTC timestamp>.sqlite`. A snapshot that fails its check is deleted and reported in Storage.
- **Kept:** the newest 14. Older ones are removed only after a newer one has passed its check.
- **Never touched** by Clean caches, by low-space handling or by any job. Low space stops new snapshots and says so; it never deletes one.
- **Restoring** in Phase 3 is by hand, documented in `docs/architecture.md`: quit, copy a snapshot over `library/knowledge.sqlite`. An in-app restore, and backups of the catalog, are Phase 4.

### 6.4 The catalog

**Migration 0014 (Slice 3.1):**

```sql
ALTER TABLE documents ADD COLUMN boilerplate_chars INTEGER NOT NULL DEFAULT 0;  -- text left out when read (§7.6)
```

Nothing else in the catalog changes. `cache/previews/` (reserved since Phase 0) holds rendered pages. Settings (`config/settings.json`) gain `privacy.recordTrails`, `privacy.expireTrails` and `export.lastFolder`, with defaults that keep old files valid.

## 7. Behaviour

### 7.1 Recording a trail (`knowledge/record.rs`)

- **What makes a step:**
  - **A search:** a submit in Find sources. Paging, a live refresh, a filter change alone and returning to the mode are not steps. The same words and filters as the trail's last step update that step.
  - **A question:** recorded when its answer ends, whether done, stopped, failed or with nothing found.
  - **A source opened:** the details panel, the page view, Open or Show in the file manager, when reached from a result, a citation, Related, a suggestion, a bookmark or a trail. Opening from the Library is not a step.
  - **A note** the user adds to the trail.
- **Recording is done in Rust.** The renderer never writes a step's content:
  - `ask_atlas` records at the end of the stream, from the answer service's own state;
  - `search_documents` takes an `intent` (`submit` or `refresh`) and records on `submit`;
  - opens go through `record_open(fileId, via, place?)`, and Rust builds the anchor from the catalog.
- **Payloads** are typed, versioned JSON:
  - `search`: words, filters (with the folder's name as it was), matched count, the top five results as anchors with their reasons, the meaning status.
  - `ask`: question, filters, the answer text, how it ended, cited and dropped, weak, the meaning status, the model's name and where its service was (this computer, another computer, an internet service), and the sources: **cited ones with their excerpts, uncited ones as anchors without**. About 4 KB a question.
  - `open`: the anchor, how it was reached, what was done.
- **The current trail** is the one whose last step was under 30 minutes ago, or the one the user continued, or a new one. It survives a relaunch within that time.
- **Paused** (`privacy.recordTrails` off): nothing is written. Asking, searching and saving all work; a finding can still be saved from the answer on screen.
- **Expiry:** on opening a library and daily, trails that are unpinned, in no collection and whose last step is over 90 days old are deleted with their steps. Findings and bookmarks never expire. With `privacy.expireTrails` off, nothing expires.
- **A step is one transaction.** A recording failure never fails the search or the answer: it is logged and shown once in the indicator ("Atlas couldn't save this step").

### 7.2 Resuming (`knowledge/trails.rs`)

- `get_trail` returns the steps with every anchor resolved (§6.2) against the catalog now.
- **A recorded answer is shown from the store,** not asked again. Its chips open the anchors of the sources as they were sent.
- **Search again and Ask again** run now, against the library and models as they are now, and add new steps to the continued trail. The old step stays as it was.

### 7.3 Bookmarks and findings (`knowledge/saved.rs`)

- **A bookmark** is created from a file id and an optional place; Rust reads the document, its content hash and the passage from the catalog and builds the anchor. A second bookmark of the same place in the same document updates the first.
- **A finding from an answer** is built in Rust from the answer service's last ended answer (kept in memory until the next question), or from a recorded step. The renderer sends an answer id or a step id, never the text.
- **A finding from a passage** is built like a bookmark, with the passage as its text.

### 7.4 Interests, projects and rediscovery (`knowledge/{collections,rediscovery}.rs`)

- **Suggestions** are computed, never stored:
  - **Seeds:** each active collection's documents (its own, and those of its bookmarks, findings and trails), and the three most recent trails' documents.
  - **For each seed,** the documents nearest the mean of its documents' stored vectors, through Related's index access (`services/documents/related.rs`), above the model's similarity floor.
  - **Left out:** the seed's own documents, any document in `step_documents`, dismissed ones, and documents with no available file.
  - **Chosen** round-robin across seeds, up to the limit, in an order fixed for the day.
- **No model is called:** only vectors already stored are read. With no meaning index, or no seed, there are no suggestions and Home's section is hidden.
- **Not for this** records a dismissal for that collection (or for recent trails).

### 7.5 The page view (`services/reader/`)

- **A second worker.** The reader owns its own `Supervisor` over the same worker executable (`WorkerCommand::current_exe`), started at the first render and killed after 60 seconds idle or when the library closes. Extraction requests never go to it, and render requests never go to the extraction worker, so neither waits for the other.
- **Protocol version 2** adds one request and one frame:
  - `Render { path, page, width, find? }`: the page (1-based), the pixel width wanted, and optionally the text to find.
  - `Rendered { page, pageCount, width, height, png, rects, found }`: the image as base64, and the rectangles of the found text in pixels.
  - A version-1 supervisor never sends `Render`; a worker answers an unknown request with `unknown_request`, as now.
- **Limits:** width at most 2,000 px, the long edge at most 3,000 px, the PNG at most 8 MiB, a 30-second inactivity timeout and the extraction worker's memory cap. Over a limit, the worker is killed and the view says it couldn't show the page.
- **Finding the passage:** the first 200 characters of the excerpt, whitespace collapsed, searched on the page with PDFium's text search; if not found, the first 60. Not found means no outline, never an error.
- **The outline is drawn by the app** over the image from `rects`, not burned into it, so it follows the theme.
- **The cache:** `cache/previews/<content hash>/<page>-<width>.png`, at most 200 MB, oldest files removed first. It is keyed by content, so a moved file's pages are reused. Clean caches empties it.
- **Checks before rendering** are Open's (`services/source_open`): the folder is reachable, the path is a regular file inside its root, and a file only in the cloud needs the user's agreement first.
- **The image crosses IPC as base64** in `PageImage` and is shown through a `data:` URL, which the CSP's `img-src 'self' data:` already allows. The CSP does not change.

### 7.6 The carry-overs (Slice 3.1)

- **Files that come back from the cloud are read.**
  - A rescan that finds a file with `extract_state = 'skipped_cloud_only'` now stored locally sets it back to `pending`, and the extraction job is queued for that folder.
  - Today only a change of size or modification time resets it (`services/inventory/db.rs`), and a download changes neither.
- **The weak margin belongs to the model.**
  - `services/providers/recommendations.json` gains `weakMargin` per embedding model; `QueryProfile` carries it.
  - A model the catalog doesn't know gets 0.20, today's constant.
  - `answers/retrieve.rs` uses the profile's value.
- **Project Gutenberg's header and licence are left out when a book is read.**
  - The EPUB reader drops the text before a `*** START OF THE (or THIS) PROJECT GUTENBERG EBOOK … ***` line and from the `*** END OF …` line on. Spine items that are wholly outside produce no passages.
  - The dropped length is recorded in `documents.boilerplate_chars`.
  - `READER_VERSION_EPUB` goes from 2 to 3, so affected books are re-read under ADR 0011: they stay searchable meanwhile, and only chunks whose text changed are embedded again.
  - A book with no such lines is read exactly as before.
  - `answers/retrieve.rs` keeps its own check as a guard.
  - This is one publisher's known markers, not general boilerplate detection.

### 7.7 Export and import (`services/export/`)

- **Markdown layout,** inside `<chosen folder>/Atlas/`:
  - `Trails/<date> <title>.md`: a trail, step by step, with answers, their sources and notes.
  - `Findings/<date> <first words>.md`: a finding with its sources and note.
  - `Bookmarks/<document title>.md`: a document's bookmarks.
  - `Interests/<name>.md` and `Projects/<name>.md`: a collection's page, linking its notes.
- **Every note starts with front matter:** `atlas_id`, `atlas_kind`, `created`, `updated`, `exported`. Links between notes are `[[Trails/…]]` or relative Markdown links, as chosen. A source is written as its title, its page or location, and the file's path as text.
- **Where Atlas may write.** This is the one place Atlas writes outside its data root (ADR 0019, added to ADR 0001's list of exceptions):
  - The folder comes from a native dialog shown by Rust. The renderer never supplies a path.
  - Atlas writes only inside the `Atlas` folder it creates there. An existing `Atlas` that is a file or a symlink is refused.
  - A folder inside a library folder is refused (originals are read-only), as is one inside the data root other than `exports/`.
  - **A file is replaced only if its front matter carries the same `atlas_id`.** Any other existing file is left alone and the note is written beside it with a number. Atlas never deletes a file there.
  - Names are made safe for all three systems (reserved names, forbidden characters, length), and each file is written to a temporary name in the same folder and renamed.
- **JSON** is one file, `Atlas/atlas-knowledge-<date>.json`: a schema version and every row of the store, anchors included. Hashes are hex.
- **Import** reads a JSON export chosen through a native file dialog: at most 256 MiB, a known schema version, every row validated like user input. A dry run reports what is new and what is already here; applying it adds new items and, for an item already here, keeps the newer of the two. It never deletes.
- **`Exporter`** is the trait product spec §10 names, with the two built-in implementations. Nothing loads exporters at run time.

## 8. Privacy and security

- **Trails are the most personal data Atlas holds:** what the user wondered about, and what a model answered. They are stored only in the knowledge store, on this computer. Nothing in Phase 3 sends them anywhere; they leave only through Export, to a folder the user picked.
- **ADR 0015 is amended (ADR 0017):** "nothing is persisted" becomes "an answer is persisted only as a trail step or a finding, in the knowledge store, and only while recording is on or because the user saved it". The catalog, the logs and the job tables still never hold a question or an answer.
- **Logs** record counts and timings, never words searched, questions, answers, notes or excerpts.
- **The Phase 2 gate is unchanged.** Recording, resuming and suggestions contact no service. Search again and Ask again are ordinary searches and questions, under the same privacy rules, disclosure included.
- **Rendering untrusted PDFs** stays in a killable process with a timeout and a memory cap. The main process still never loads PDFium. The renderer receives a PNG, never a path.
- **Imported files are untrusted:** size-capped, schema-checked, and never able to name a path or run anything.
- **Exported notes are the user's:** Atlas says where it wrote them and does not track them afterwards.

## 9. Service interfaces and extension-ready design

```rust
// Earlier traits stand. Phase 3 adds:
struct KnowledgeStore;        // open/migrate, one writer (KnowledgeHandle::call), a read pool, snapshot
struct Anchor;  enum AnchorState { Found, FoundElsewhere, NoFile, Unknown }
fn resolve(catalog: &Connection, anchor: &Anchor) -> Resolved;      // never writes
trait Clock: Send + Sync { fn now(&self) -> OffsetDateTime; }        // SystemClock in the app; tests set their own
struct Recorder;              // search/ask/open steps into the current trail; a no-op while paused
struct ReaderService;         // its own Supervisor; render(file, page, width, find) -> PageImage; the cache
trait Exporter { fn id(&self) -> &'static str; fn export(&self, set: &ExportSet, dest: &ExportDest) -> Result<ExportReport, ExportError>; }
// MarkdownExporter and JsonExporter; tests inject a destination in a temporary folder.
```

New typed IPC commands (each registered in the slice that implements it; `tests/ipc_contract.rs` forbids stubs):

```text
get_knowledge_status() -> KnowledgeStatus                           # 3.2: counts, size, snapshots, recording
search_documents(query + intent)                                    # 3.3: gains intent: submit | refresh
record_open(file_id, via, place?) -> ()                             # 3.3
list_trails(query) -> TrailPage                                     # 3.3
get_trail(trail_id) -> TrailDetail                                  # 3.3: steps, anchors resolved
start_trail() -> TrailSummary;  continue_trail(trail_id) -> TrailSummary          # 3.3
rename_trail, pin_trail, delete_trail, delete_trail_step, add_trail_note, delete_all_trails   # 3.3
save_bookmark(input) -> Bookmark;  save_finding(input) -> Finding   # 3.5: input names a file and place, an answer id or a step id
update_saved_note, delete_saved, list_saved(query) -> SavedPage     # 3.5
list_collections, get_collection, save_collection, delete_collection,
add_to_collection, remove_from_collection                           # 3.6
get_suggestions(limit) -> Vec<Suggestion>;  dismiss_suggestion(...) # 3.6
render_page(file_id, page_no, width_px, find?, allow_download) -> PageImage       # 3.7
export_knowledge(scope, format, link_style) -> ExportReport         # 3.8: Rust shows the folder dialog
preview_import() -> ImportPreview;  apply_import(token) -> ImportReport           # 3.8: Rust shows the file dialog
```

**DTOs.** All ts-rs exported; `check-bindings` covers them. Recording and expiry are fields of `Settings`, saved through `update_settings`.

**Events.** `trail-changed` (the current trail and its step count, coalesced) keeps the indicator and an open timeline current.

**Security boundary.** The renderer sends ids, never paths and never a step's content. A note, a title, a name and a goal are the only free text it sends: length-capped and stored as text. Every list is paged and size-capped. `render_page` validates the page and width in Rust. Export and import paths come only from dialogs shown by Rust.

## 10. Evaluation: the resume scenario

`fixtures/knowledge/resume-scenario.json` is a committed script that `tests/resume.rs` runs on every OS against the spike corpus and the scripted model service:

1. **A session:** three searches, two questions (one answered with citations, one stopped), four sources opened, a bookmark, a finding, an interest holding them.
2. **The clock moves 12 weeks.** The trail is pinned, so it is kept; a second, unpinned trail is not.
3. **The library changes:**
   - a cited file is moved to another folder;
   - another is renamed;
   - one is deleted;
   - one book is re-read by a newer reader version, which re-splits its passages;
   - the embedding model is switched.
4. **The app is restarted.**
5. **Expected:** every step is shown in order, and the recorded answer's text is identical. Each anchor resolves to the state the script names: `found` for the moved and renamed files, `found_elsewhere` for the re-read book, `no_file` for the deleted one. The bookmark, the finding and the interest are intact. Search again and Ask again add steps to the continued trail.

**Search quality must not fall** with the Gutenberg cut: P2-17's fake-backend floor holds in CI. The product owner re-runs `just eval-semantic` and `just eval-ask` after Slice 3.1, and the results go in `docs/benchmarks.md` beside Phase 2's.

## 11. Test fixture collection

- `scripts/generate-knowledge.mjs`: a knowledge store of a given size (the benchmark uses 10,000 trails and 100,000 steps, with bookmarks, findings and collections in proportion), written through the same JSON import format, so it also tests import.
- **Fixture PDFs with known text positions:** the generator's PDFs (`scripts/lib/fixture-text.mjs`) start each page's text at a fixed point with a fixed line height, and the file's marker phrase is in the first sentence. The highlight test checks the rectangles against that.
- **The spike corpus's Project Gutenberg EPUBs** are the boilerplate fixtures. The malformed PDFs (`corpus/malformed/`) are the render-failure fixtures.
- `e2e/fake-ai.mjs` is unchanged; the smoke's resume steps use its canned answer.

## 12. Required automated tests

| ID | Scenario | Expected result |
| --- | --- | --- |
| P3-01 | A file cataloged while only in the cloud, then stored locally with the same size and time, rescanned | It goes back to waiting and is read; a file still in the cloud is not opened |
| P3-02 | The weak margin from the model's profile; a model the catalog doesn't know | The profile's value decides `weak`; the default is 0.20 |
| P3-03 | The corpus's Project Gutenberg EPUBs read by reader version 3; a book without the markers | No passage holds the header or the licence; `boilerplate_chars` is recorded; the unmarked book's passages are unchanged; books stay searchable during the re-read, and unchanged chunks are not embedded again |
| P3-04 | The knowledge store: created on first open; migrations; a store from a newer Atlas | Opens or refuses with a clear error; never downgraded or altered |
| P3-05 | SIGKILL during a step write, a bookmark write and a collection change | Whole rows only; `integrity_check` passes; the catalog is unaffected |
| P3-06 | Snapshots: made when the store changed, at most daily; a kill during one; a snapshot that fails its check; the fifteenth | No partial file; the failed one is removed and reported; the oldest goes only after a newer one passed |
| P3-07 | Clean caches, low space and every job, with a full knowledge store and snapshots | Neither is touched; the page cache is emptied by Clean caches |
| P3-08 | Anchors: the file moved; renamed; a copy elsewhere; the document re-read by a newer reader; the file changed; the folder removed; the catalog rebuilt | `found`, `found`, `found`, `found_elsewhere`, `no_file`, `no_file`, `unknown` then `found` once read; resolution never writes |
| P3-09 | Recording: a submit, paging, a live refresh, a filter change, the same search twice; a question done, stopped, failed and with nothing found; opens from each origin and from the Library | Exactly the steps §7.1 names, in order, each in one transaction |
| P3-10 | Paused recording; a recording failure (the store read-only) | Nothing is written while paused; the search and the answer still succeed and the failure is reported once |
| P3-11 | Sessions: a step after 29 and after 31 minutes; New trail; Continue; a relaunch inside the window | The same trail, a new one, a new one, the continued one, the same one |
| P3-12 | Expiry at 90 days: unpinned, pinned, in a collection, expiry switched off; its findings and bookmarks | Only the unpinned trail outside a collection goes; findings and bookmarks stay |
| P3-13 | **The resume scenario** (§10) | Every expectation of the script |
| P3-14 | Deleting a step, a trail and all trails; a finding saved from a deleted step | Gone from the store and from collections; the finding stays whole |
| P3-15 | Bookmarks and findings: saved from a result, a citation, a page and an answer; the same place twice; a note at the limit and over it | Anchors built in Rust from the catalog; one bookmark per place; the note is capped |
| P3-16 | No words, questions, answers, notes or excerpts in the logs, the catalog or the job tables after a recorded session | Only counts and timings |
| P3-17 | Collections: items of each kind; deleting an item; finishing and archiving; deleting a collection | Items leave their collections; a deleted collection never deletes its items |
| P3-18 | Suggestions: seeds from a collection and from recent trails; an opened document; a dismissed one; a missing file; no meaning index | Those are left out; the order is stable for the day; no request reaches any service; none without an index |
| P3-19 | Rendering: a page of a fixture PDF with its marker found; an excerpt that isn't on the page; a scanned page; the last page plus one | An image with rectangles around the marker; an image without; an image without; a clear refusal |
| P3-20 | Rendering limits: a malformed PDF; a page over the size limits; a worker that hangs | The reader's worker is killed and restarted; the extraction job, running at the same time, is undisturbed; the main process never loads PDFium |
| P3-21 | The page cache: a second request; a moved file; over 200 MB; a cloud-only file | Served from the cache; reused by content; the oldest removed; not rendered without agreement |
| P3-22 | Markdown export: each kind, both link styles; names with reserved and long forms; a second export | Notes with front matter and working links; safe names; files with the same `atlas_id` replaced, others left alone |
| P3-23 | Export destinations: inside a library folder; inside the data root; an `Atlas` that is a file or a symlink; a symlink planted for a note's name | Refused with a reason, or written beside; nothing outside `<chosen>/Atlas/` is ever created, changed or deleted |
| P3-24 | JSON export then import into an empty store and into the same store; a newer item; a truncated file, an unknown version and an oversized file | The stores are equal; nothing duplicated; the newer one wins; each bad file is refused whole and nothing is written |

Use the injected fakes of earlier phases (the volumes fake that marks files cloud-only, as in P1-13; the scripted HTTP server; `HashedBagOfWordsBackend`) and the new settable clock. Add migration tests for 0014 and knowledge/0001, integrity checks on both databases after every crash test, and the originals-unchanged snapshot to every test that reads files.

## 13. Benchmarks and hardware validation

Record performance, don't invent guarantees. At minimum measure:

- **The knowledge store** (`benches/knowledge.rs`, 10,000 trails and 100,000 steps): the Trails list and its filter, opening a 50-step trail with its anchors resolved, the cost recording adds to a search and to an answer, a snapshot, the store's size per step.
- **The page view** (`benches/reader.rs`, the corpus's PDFs): the first render after the worker starts, a render at 1,200 px (p50, p95), with and without the highlight, a cache hit, the worker's peak memory.
- **Export:** 1,000 notes as Markdown, the whole store as JSON, and an import of it.
- **The carry-over:** the re-read and re-embedding of the corpus's Gutenberg books (how many chunks changed).
- macOS reference machine first; Windows and Linux CI evidence before sign-off.

Provisional targets, to report against and not to publish as requirements: **recording adds under 5 ms** to a search or an answer; **the Trails list p95 under 100 ms** at 100,000 steps; **a trail of 50 steps opens in under 200 ms**; **a page renders in under 500 ms p95** at 1,200 px on the reference machine; **a snapshot takes under 2 s** at 100,000 steps.

## 14. Cross-platform packaging, CI and release evidence

- **Licences.** `docs/dependency-licenses.md` gains `png` as a direct dependency; GPL/AGPL findings flagged as before.
- **Bundles.** Record the growth since 0.3.0 in `docs/packaging.md`. PDFium is already bundled; rendering adds no native library.
- **CI.** The full tier runs the new tests. The bench job adds `benches/knowledge.rs` and `benches/reader.rs`.
- **Smoke** (Windows and Linux, after Phase 2's steps): "a search and a question are recorded", "quit and relaunch", "the trail is resumed and its citation opens", "a bookmark is saved", "the page view shows a page", "a trail is exported to a scratch folder" (a debug-only hook answers the folder dialog, as for the pickers).
- **Release evidence.** The DMG smoke on macOS by the product owner against oMLX, with a real session resumed from the release build, per a checklist in `docs/platform-validation.md`.
- The artifacts still start without Python, Docker, a database server, Ollama or an LLM. Without an AI service, trails record searches, and bookmarks, interests, the page view and export all work.

## 15. Implementation backlog: ordered vertical slices

Implement and verify **one slice at a time**. Build and rerun tests after each slice, then proceed. Never claim later slices are done because scaffolding compiles.

| Slice | Deliverable | Verification gate |
| --- | --- | --- |
| 3.1 | **Carry-overs.** The iCloud re-read fix; `weakMargin` per model; Project Gutenberg boilerplate at extraction (migration 0014, EPUB reader version 3); slice strings to Phase 3 | P3-01, P3-02, P3-03; P2-17's floor in CI; the owner's `eval-semantic` and `eval-ask` recorded |
| 3.2 | **The knowledge store.** `knowledge.sqlite`, its migrations and writer; anchors and resolution; snapshots; the Storage category; ADR 0016 | P3-04, P3-05, P3-06, P3-07, P3-08 |
| 3.3 | **Recording.** Steps for searches, questions and opens; sessions; pause; expiry; deletion; the indicator; Privacy › Research trails; ADR 0017 (amends ADR 0015) | P3-09, P3-10, P3-11, P3-12, P3-14, P3-16 |
| 3.4 | **The Trails screen and resuming.** The fifth view, the list and timeline, Continue, Search again, Ask again; Home's recent trails | P3-13; Vitest for every screen state |
| 3.5 | **Bookmarks and saved findings.** The Save actions, the Saved tab, notes, Bookmarks in this document | P3-15; Vitest |
| 3.6 | **Interests, projects and rediscovery.** Collections, Add to…, the collection page, suggestions, Home's other two sections | P3-17, P3-18; Vitest |
| 3.7 | **The PDF page view.** The reader's worker, `Render` (protocol 2), the highlight, the cache, the view; ADR 0018 | P3-19, P3-20, P3-21; Vitest; `benches/reader.rs` |
| 3.8 | **Export and import.** Markdown to a chosen folder, JSON, import with a dry run; ADR 0019 and ADR 0001's amended list | P3-22, P3-23, P3-24; the smoke's resume and export steps on Windows and Linux |
| 3.9 | **Evidence and close-out.** Benchmarks on three OSes; licence report; the DMG smoke with a resumed session; `docs/phase3-closeout.md`; version 0.4.0 | All CI green on `main`; review against §1, §12, §13, §14 and §16 |

**Ordering notes.** 3.1 is independent and first, so its re-read happens before anything is anchored to passages. 3.3 to 3.6 need 3.2. 3.4 needs 3.3. 3.7 needs only 3.2 (for the cache and anchors) and can move earlier if the product owner wants the page view sooner. 3.8 needs 3.3, 3.5 and 3.6.

Each slice includes: code, automated tests, a README and docs update (`architecture.md`, the ADRs, `benchmarks.md`, `platform-validation.md`, `packaging.md` and the justfile as affected), explicit demo steps, and a brief note of any deviations from this specification. Checkpoints use the established convention: a `slice-3N` branch, squash-merged onto `main` as `phase3/slice-0N: …` with an annotated tag `phase3/slice-0N`.

## 16. Acceptance checklist for the product owner

- [ ] Weeks after a research session, I can open its trail and see what I searched, what I asked, what Atlas answered and what I opened, without having saved anything myself.
- [ ] A citation in an old answer still opens the right document at the right place, even though I moved the file.
- [ ] When a file is gone, Atlas says so and still shows me the passage I saw.
- [ ] I can pause recording, delete a step, a trail or everything, and nothing is recorded while paused.
- [ ] I can bookmark a place and keep an answer or a passage with a note of my own.
- [ ] I can group things under an interest or a project, and I never have to.
- [ ] Home shows where I left off and suggests things from my library I haven't opened.
- [ ] A citation or bookmark in a PDF shows me the page itself, with the passage marked.
- [ ] I can export a trail, a finding or everything as Markdown into a folder I choose, and Atlas never overwrites my own notes there.
- [ ] I can export everything as one file and import it again.
- [ ] Cleaning caches, a crash or a damaged catalog never costs me my trails and notes.
- [ ] A book that was only in iCloud is read once I download it.
- [ ] Recorded test, benchmark, licence and platform-smoke evidence for the three OSes.

## 17. Constraints for the coding agent

1. **Read this entire spec, the Phase 2 spec, the Phase 2 close-out review, ADRs 0001, 0009, 0011, 0013 and 0015, and Atlas Product Specification v0.2 first.** The parent spec is product direction; this document is the Phase 3 scope and acceptance contract.
2. Work incrementally, beginning with Slice 3.1. Deliver working code rather than architecture-only commentary. Prefer maintained existing libraries over custom replacements.
3. If implementation conflicts with a stated invariant (read-only originals, recoverable jobs, no mandatory AI, no silent network calls, keys only in the keychain, the single data root with its documented exceptions), preserve the invariant and document the tradeoff.
4. **What the user creates is never deleted to recover space, never rebuilt from something else, and never the casualty of a cache clean, a job or a migration.** Deleting it takes an action by the user that names what goes.
5. **Never anchor on `passages.id`, `chunks.id` or `file_instances.id`.** Use an anchor (§6.2) and resolve it when shown.
6. **Atlas writes outside its data root only inside the `Atlas` folder of an export destination chosen in a native dialog** (§7.7), and never inside a library folder.
7. Don't overbuild future phases. No EPUB renderer, no editor, no tags, no deep links, no sync, no catalog backups.
8. Validate inputs on the Rust side. Treat document text, model output, imported files, notes and names as untrusted data. The renderer sends ids, never paths and never recorded content.
9. Use explicit migrations for both databases and structured error codes. Prefer deterministic fakes (the settable clock, the scripted service, `HashedBagOfWordsBackend`) to real waiting or real models.
10. Never show a step as recorded before its transaction committed; never show an anchor as found without resolving it now; never present a page as the cited one when the passage wasn't found on it without saying so.
11. Run formatting, static checks and tests before claiming each slice is done. Report commands executed and outcomes. Never claim a Windows, Linux or macOS build was tested unless it actually ran there; label unverifiable steps.
12. When a measurement contradicts a document, correct the document in the same change.
13. Never read the owner's API keys, never download a model, never touch the owner's real library or its knowledge store; measurement runs use scratch libraries built from the corpus and fixtures.
14. **Commits and pushes** (the owner's standing rule, 2026-11-10): commit freely on slice branches; push branches, `main` and `phase*` tags once the full CI run is green; never bypass commit signing, and stop and ask if signing fails; ask before any force-push or deletion of a branch or tag.
15. End each slice report with: implemented, tests passed, manual verification, known issues, and next slice.

## 18. Kickoff instructions for the coding agent

**First requested task:** Implement **Slice 3.1** only:

- In the inventory upsert (`services/inventory/db.rs`), reset `extract_state` from `skipped_cloud_only` to `pending` when a rescan finds the file stored locally, and queue the folder's extraction job.
- Add `weakMargin` to the embedding entries of `services/providers/recommendations.json`, carry it in `QueryProfile`, and use it in `answers/retrieve.rs`.
- Add migration 0014; drop Project Gutenberg's header and licence in `extraction/engine/epub.rs`, recording `boilerplate_chars`; raise `READER_VERSION_EPUB` to 3.
- Add P3-01, P3-02 and P3-03, and the migration test for 0014.
- Change the slice strings to Phase 3 (`CURRENT_SLICE = "3.1"`, the test mocks and `ipc_contract.rs`).
- Update `docs/architecture.md`, ADR 0011 (the reader version), ADR 0015 (the margin), `migrations/README.md`, the README status and the justfile as affected.
- Ask the product owner to re-run `just eval-semantic` and `just eval-ask`, and record the results.

After the slice passes, summarize code and verification, note deviations, and name Slice 3.2 as the next step.

Do **not** create `knowledge.sqlite`, add a Trails view or record anything before Slice 3.1's gate passes.

---

### Revision notes

**2026-11-10, v0.1** (written after the Phase 2 close-out review):

- **Product-owner decisions recorded:**
  - a PDF page view only, with EPUBs keeping the passage text;
  - trails recorded automatically, with pause, deletion and a 90-day expiry for unpinned trails;
  - Markdown export to a folder chosen each time;
  - the iCloud re-read fix, a per-model weak margin and Project Gutenberg boilerplate at extraction taken from the close-out;
  - no `atlas://` links and no fetch of newer recommendations in this phase.
- **Decided in writing this spec, for the product owner to confirm or change:**
  - **A second database** for what the user creates (`library/knowledge.sqlite`), where the product spec's proposed layout (§8) put notes and trails in the catalog. It can be snapshotted whole and often, a damaged catalog can't take it along, and it is the unit a later sync starts from. It ends "one SQLite database" as a scope rule.
  - **Anchors, not row ids** (§6.2), because passage and chunk ids don't survive a re-read and file ids don't survive a move.
  - **Snapshots of the knowledge store only**, 14 kept; catalog backups stay in Phase 4.
  - **Interests and projects as one grouping** with two kinds; no tags, no inferred topics.
  - **Notes as plain text**, 4,000 characters; no editor.
  - **Suggestions from stored vectors only**, with no model call.
  - **Trails as a fifth main view.**
  - **Uncited sources of a recorded answer kept as anchors without their excerpts**, to keep a question near 4 KB.
- **Carried from the Phase 2 close-out and still deferred:** the unexplained miss of an index's first vector on the Linux runner (worked around by the exact search of small indexes); the single Windows failure of a `related` test; the 30k fixture embedded through oMLX overnight (not run); signing and notarization.

**2026-11-11, Slice 3.7** (the PDF page view; [ADR 0018](adr/0018-the-page-view.md)):
- **Refusals are an answer, not a kill.** Protocol 2 adds
  `RenderFailed { reason }` beside `Rendered`, with reasons
  `page_out_of_range`, `encrypted`, `malformed`, `too_large`, `cloud_only`
  and `bad_path`. The worker keeps running. Only a hang, the memory cap, a
  crash or a protocol fault kill it. The command's `render_failed` error
  adds `not_read`, `not_a_pdf`, `unavailable` and `worker`; a width outside
  1 to 2,000, or page 0, is `invalid_render`.
- **The cache key includes what was looked for:**
  `cache/previews/<content hash>/<page>-<width>-<find key>.png`, with a
  `.json` beside it (page count, size, rectangles). The find key is 8 hex
  characters of the BLAKE3 hash of the normalised text, or `none`. §7.5
  had `<page>-<width>.png`, which can't hold the outline.
- **A cached page shows for a file now only in the cloud:** the cache is
  read before the file is, and nothing is downloaded to show it. P3-21's
  "not rendered without agreement" holds for pages not yet cached.
- **A page too tall at the width asked for is scaled** to the 3,000 px long
  edge, not refused, so a long page still shows. P3-20's "a page over the
  size limits" is the width over 2,000 (`invalid_render`) and the 8 MiB PNG
  limit.
- **`find` is text the screen already shows** (an excerpt or a snippet, at
  most 2,000 characters). It is searched for and hashed for the key, and
  never stored. The passage is looked for only on the page it was cited
  on.
- **The PDFium module check** (P3-20) runs on macOS (dyld) and Linux
  (`/proc/self/maps`). Windows has no cheap module list, so it is skipped
  there.
- **P3-21's cloud-only case is tested at the reader level.** It renders
  with the path's `FileCloudOnly` error in hand, because a file's cloud
  flags can't be faked on a real filesystem. `path_for`'s mapping is
  Open's, which P1-13 covers.
- **Malformed PDFs that PDFium repairs render.** P3-20 asks that each of
  the corpus's `malformed/` files either renders or gives `render_failed`,
  and that the next good render works.
- **Where the outline is missed:** text PDFium reads differently from the
  extraction. Superscript marks in a paper's author list and hyphens PDFium
  marks with a control character both do it. 4 of 37 corpus pages in the
  benchmark, all arXiv first pages or hyphenated lines. The page still
  shows, with its text below.

**2026-11-11, Slice 3.6** (interests, projects and rediscovery):
- **Field limits:** a name of 1 to 200 characters; a description and a
  goal of up to 4,000. A goal and Finished are a project's only
  (`invalid_collection`). A collection keeps its kind. A finished or
  archived one can be made active again.
- **Documents in a collection** are added by a file id and kept by
  document id and content hash. They are resolved when shown, and one the
  catalog no longer has says "Atlas hasn't read this document yet".
- **Deleting a collection** also deletes its dismissals.
- **Suggestions leave out documents opened in a trail,** not every
  `step_documents` row. Those rows also hold every search result and
  cited source, which would leave out most of what the user has merely
  seen. P3-18 names "an opened document".
- **A seed's vector** is the re-normalised mean of its documents' Related
  vectors.
- **The day's order:** seeds sorted by a hash of the date and the seed,
  then round-robin.
- **Added to §9:** `get_collection_membership(item)`, for Add to…'s toggles.
- **Still deferred:** Export on a collection (3.8).

**2026-11-11, Slice 3.5** (bookmarks and saved findings):
- **What the renderer sends:**
  - `save_bookmark(target)`: `file{fileId, place}`, where `place` is
    `result`, `citation{answerId, n}`, `page{pageNo}` or `document`; or
    `step{stepId, index}` for a recorded source.
  - `save_finding(source)`: `answer{answerId}`, `step{stepId}` or
    `passage{target}`.
- **A place** is the document and the anchor's passage ordinal; the whole
  document is a place of its own.
- **An answer finding keeps only the cited sources,** as §5.3 says ("with
  its question and cited sources").
- **`save_source_gone`** answers a save whose answer was replaced by a
  newer question, or whose step was deleted.
- **The Saved filter is matched in Rust,** folded, over titles, excerpts,
  questions, text and notes: the tables have no filter column, and a
  knowledge store's saved items are few.
- **Tabs:** Trails | Saved in 3.5; Interests and projects joins in 3.6.
- **Still deferred:** Add to an interest or project (3.6) and Export
  (3.8) on saved items; the page view's Bookmark (3.7).
- **`SavedItem` is tagged `item`** (not `kind`, which a finding has).

**2026-11-11, Slice 3.4** (the Trails screen and resuming):
- **Tabs and actions arrive with their slices.** The Trails | Saved |
  Interests and projects tabs come in 3.5, with the second tab, so none is
  empty. Add to an interest or project (3.6), Export (3.8) and Save as a
  finding (3.5) join the timeline then.
- **A recorded folder no longer in the library** is dropped by Search again
  and Ask again, and the Search screen says so once.
- **What each source says** (§5.1's sentences, completed):
  - `found_elsewhere`: "This passage has moved within the document since."
  - An excerpt not found again: "Atlas couldn't find this passage again."
  - A question's source opens with its excerpt as sent: "Sent to the model
    for this question on {date}."
- **P3-13** (`fixtures/knowledge/resume-scenario.json`):
  - It writes the bookmark, finding and interest rows directly until 3.5
    and 3.6 bring their services.
  - Its earlier trail starts 7 days before the session, so 12 weeks is
    past the 90 days.
  - The re-read is real: the book is put back to reader version 2 with a
    Project Gutenberg header passage, so reader 3 re-reads it and every
    passage moves one place.
  - The changed files are found by a rescan after the restart: Atlas
    doesn't watch folders.
- **Found by P3-13 and fixed:** a scan that finished while the reading job
  was re-reading books queued no job, and that job completed without
  reading the files the scan had found. It now reads them before it
  completes.

**2026-11-11, Slice 3.3** (recording trails,
[ADR 0017](adr/0017-trails-and-privacy.md)):
- **`start_trail` makes no empty trail.** It clears the current trail so
  the next step starts one, and returns the `RecordingStatus`, not a
  `TrailSummary`.
- **`record_open(fileId, via, action, place?)`** gains `action` (details,
  page, open, reveal). `via` carries a citation's answer id and number, so
  Rust anchors at the passage that was cited. `library` is a `via` that
  records nothing.
- **Opens merge:** the same document opened again the same way (the
  details, then Open) adds the action to the last step.
- **A note the user adds is written while paused**, like a finding saved
  from the screen. Only automatic recording pauses.
- **Not steps:** a question refused before anything was asked (no chat
  model, external AI off, an invalid question) and the disclosure. "Failed"
  in P3-09 is an answer that failed after it started.
- **Atlas has no log.** "Logged and shown once" (§7.1) is shown once: the
  status's `failed` flag, until a step is saved again. P3-16 checks the
  process's stdout and stderr, every file of the library and its config
  outside the store, and the catalog's tables.
- **`trail-changed`** is sent after each step and each change, not
  coalesced: steps come at the user's pace.
- **Added to §9:** `get_recording_status` and `dismiss_trail_note` (Home's
  note, `meta.first_step_noted`), and the error codes `trail_not_found`
  and `text_too_long`. The recording state is its own `RecordingStatus`,
  not part of `KnowledgeStatus`.
- **A search step's results** keep a 300-character excerpt each, enough to
  find them again after a re-read; a cited source keeps its excerpt as
  sent (at most 2,000).
- **`list_trails` and `get_trail`** are registered in this slice, as §9
  says; the Trails screen that uses them is Slice 3.4.

**2026-11-11, Slice 3.2** (the knowledge store, [ADR 0016](adr/0016-the-knowledge-store.md)):
- **A store that can't be used never blocks the library.** A newer store
  (`incompatible`), or a file that isn't one (`damaged`), is left byte for
  byte. The library opens, and every use of the store is refused with
  `knowledge_unavailable`. P3-04's "refuses with a clear error" is read
  this way: the store is refused, not the library.
- **Changes are counted by triggers** (`meta.changes`) on every user
  table, so a later slice can't forget to mark one.
- **The snapshot rule:** changed since the last snapshot, and the newest
  at least 24 hours old. Checked at open and hourly while open ("once a
  day" in §6.3 is the most it happens).
- **Pruning:** the oldest goes only after a newer snapshot passed its
  check, so a kill between the two leaves 15 until the next one.
- **Resolution details** that §6.2 left open:
  - An excerpt that isn't found anywhere gives `found` at the saved place
    with `excerptFound: false`.
  - When the whole excerpt isn't found, its first 160 characters are
    tried, since a re-split passage may hold only part of the old one.
- **The writer is the catalog's own code,** opened under another thread
  name (`CatalogWriter::open_named`). The plan's generic
  `sqlite_writer.rs` wasn't needed.
- **`StorageUsage`** gains `knowledgeBytes`, `snapshotBytes` and
  `snapshotCount`. `get_knowledge_status` adds the state, the counts and
  the last outcome.

**2026-11-10, Slice 3.1.1** (after the owner's `eval-semantic` on Slice
3.1):
- **The best meaning match counts twice in hybrid fusion.** Removing the
  licences took away The Raven's and the Dao De Jing's only word matches
  for v06 and v14. Hybrid hit@5 fell from 0.950 to 0.900, though meaning
  alone still ranked both first.
- **`just eval-semantic`** now says which variables it needs.

**2026-11-10, amended during Slice 3.1:**

- **Only marked EPUBs are read again.** A done EPUB already at reader
  version 2 is re-read only if the full-text index finds "project
  gutenberg" in it; every other EPUB moves to version 3 without a read.
  §7.6 implied every EPUB would be read again.
- **Notes inside the book stay.** Text after START that mentions Project
  Gutenberg, such as The Raven's "This ebook was one of Project Gutenberg's
  early files", is the book's: only the header before START and everything
  from END are cut.
- **The hold before START** is bounded at 3 spine items or 64 KiB. Past
  that, nothing is cut.
- **A downloaded file is read at the next Rescan** of its folder (Settings ›
  Library folders), not by itself: Atlas has no filesystem watcher. The
  file's detail now says so.
- **P3-03's embedding check** (only changed chunks embedded again) rests on
  the existing `vectors::p2_04_unchanged_text_is_never_sent_again_and_changed_text_is_requeued`,
  rather than a new test.

### Primary reference documentation (verify current versions at implementation time)

- pdfium-render (rendering and text search): <https://docs.rs/pdfium-render/0.9.5/pdfium_render/>
- SQLite `VACUUM INTO` and `PRAGMA integrity_check`: <https://www.sqlite.org/lang_vacuum.html>, <https://www.sqlite.org/pragma.html#pragma_integrity_check>
- The `png` crate: <https://docs.rs/png/latest/png/>
- Tauri dialog plugin (Rust API): <https://docs.rs/tauri-plugin-dialog/latest/tauri_plugin_dialog/>
- Obsidian links and properties (front matter): <https://help.obsidian.md/links>, <https://help.obsidian.md/properties>
