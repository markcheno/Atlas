# Atlas architecture

Status: Phase 1, Slice 1.6 (metadata quality, the duplicates view and
Settings › Storage, over Slice 1.5's Search screen and detail panel). Phase 0 is complete; the rest of Phase 1 is
specified in [phase1-spec.md](phase1-spec.md) and not yet built.
This document describes what is built, and marks what is only planned. Decisions with lasting
consequences are recorded as ADRs in [adr/](adr/).

## Process model

Atlas is one desktop process. The Tauri 2 runtime hosts the native window, an
OS webview that renders the Svelte UI, and the Rust core. There is no HTTP
server, background daemon, database server, or Python runtime. Text
extraction runs in a child process, the same executable started as the
[extraction worker](#extraction-worker), so PDFium never runs in the app
process.

```text
┌──────────────────────── Atlas process ────────────────────────┐
│  OS webview (WKWebView / WebView2 / WebKitGTK)                 │
│    Svelte UI ── src/lib/api/client.ts (typed facade)           │
│                         │  Tauri IPC (invoke)                  │
│  Rust core              ▼                                      │
│    commands/   thin adapters: validate input, map DTOs         │
│        │                                                       │
│    services/   workspace · catalog · jobs · inventory          │
│                scheduler · system · providers · source_open    │
│        │                                                       │
│    domain/     DTOs (ts-rs exported) and service traits        │
│    infrastructure/  volume inspector, atomic writes, clock     │
│                                                                │
│    catalog writer thread ── the only read-write SQLite conn    │
└────────────────────────────────────────────────────────────────┘
```

Tauri IPC is the private local API. Services are written so they do not
depend on Tauri types; a future, opt-in, authenticated, versioned HTTP API can
call the same service functions. See
[ADR 0001](adr/0001-local-ipc-no-server-single-data-root.md).

## IPC contract

Every Phase 0 command (spec section 9) is registered. Slice 0.2 added the
data-root and settings commands; Slice 0.3 implemented the source-root
commands and added the native multi-folder picker.

| Command group | Commands | Delivered by |
| --- | --- | --- |
| App | `get_app_status` | **0.1 (working)** |
| Data root | `get_data_root_status`, `get_default_data_root`, `pick_folder`, `inspect_data_root`, `choose_data_root`, `open_data_root` | **0.2 (working)** |
| Settings | `get_settings`, `update_settings` | **0.2 (working)** |
| Device | `get_device_snapshot`, `get_device_check` | **0.4 (working)** |
| Source roots | `list_source_roots`, `add_source_root`, `remove_source_root`, `pick_and_add_source_roots` | **0.3 (working)** |
| Inventory jobs | `start_inventory`, `pause_job`, `resume_job`, `cancel_job`, `get_inventory_status` | **0.5 (working)** |
| Library | `list_documents`, `open_original`, `reveal_original` | **0.6 (working)** |
| Providers | `list_provider_profiles`, `save_provider_profile`, `delete_provider_profile`, `test_provider_connection`, `detect_local_providers` | **0.7 (working)** |
| Model roles | `test_model_capability`, `get_model_roles`, `set_model_role`, `clear_model_role`, `get_model_recommendations` | **2.1 (working)** |
| Text reading | `get_extraction_status`, `start_extraction` (pause, resume and cancel share the job commands) | **1.3 (working)** |
| Meaning index | `get_embedding_status`, `start_embedding` (pause, resume/retry and cancel share the job commands) | **2.3 (working)** |
| Search | `search_documents` | **1.4 (working)** |
| Documents | `get_document_detail` (by file ID: a file that hasn't been read has no document yet) | **1.5 (working)** |
| Duplicates | `list_duplicates` | **1.6 (working)** |
| Storage | `get_storage_usage`, `clean_caches` | **1.6 (working)** |
| Related documents | `get_related_documents` | **2.5 (working)** |
| Ask Atlas | `ask_atlas`, `cancel_answer`, `acknowledge_disclosure` (the privacy choices are part of `update_settings`); the `answer-stream` event | **2.6 (working)** |
| Knowledge store | `get_knowledge_status` | **3.2 (working)** |
| Trails | `record_open`, `get_recording_status`, `dismiss_trail_note`, `list_trails`, `get_trail`, `start_trail`, `continue_trail`, `rename_trail`, `pin_trail`, `delete_trail`, `delete_trail_step`, `add_trail_note`, `delete_all_trails`; `search_documents` gains `intent`; the `trail-changed` event (recording and expiry are part of `update_settings`) | **3.3 (working)** |
| Saved | `save_bookmark`, `save_finding`, `update_saved_note`, `delete_saved`, `list_saved` | **3.5 (working)** |
| Interests and projects | `list_collections`, `get_collection`, `save_collection`, `delete_collection`, `add_to_collection`, `remove_from_collection`, `get_collection_membership`, `get_suggestions`, `dismiss_suggestion` | **3.6 (working)** |
| Page view | `render_page` | **3.7 (working)** |
| Export and import | `export_knowledge` (Rust shows the folder dialog), `preview_import` (the file dialog and a dry run), `apply_import`; `reveal_export` (3.9: the last export's folder in the file manager) | **3.8 (working)** |
| Diagnostics | `get_diagnostics_status`, `save_diagnostics_report`, `reveal_diagnostics` (the newest report or the log's folder), `clear_log` | **4.1 (working)** |

`get_app_status` returns only facts about the running process, including a
feature list that marks exactly which capabilities this build provides.
Commands whose slice has not shipped return a `not_implemented` error
naming itself and its planned slice.

Implemented commands are thin. They hand the work to a service on a blocking
thread (`AppState::with_workspace`), because data-root operations do
filesystem and SQLite I/O that must not run on the async executor.

**Single source of truth for types.** DTOs are defined once in
`src-tauri/src/domain/dto.rs`. `ts-rs` writes matching TypeScript to
`src/lib/api/bindings/` during `cargo test`, and CI fails if the committed
bindings drift. Conventions: camelCase fields, snake_case enum values,
RFC 3339 timestamps, opaque UUID string IDs, byte counts as JSON numbers, and
`null` for absent values.

**Contract tests.** `src-tauri/tests/ipc_contract.rs` checks that every
command is registered, listed in `COMMAND_NAMES`, invoked by the frontend
facade under the same name, and that no stub returns data.

**Errors.** One `AtlasError` type serializes to
`{ code, message, recovery?, command?, plannedSlice? }`. Codes are stable
snake_case strings, listed in `src-tauri/src/error.rs`. Every user-facing
code carries a `recovery` hint, and messages never include file paths.
The frontend adds two client-side codes:
`ipc_unavailable` when running outside the desktop app, and `unexpected`
for any malformed error, which never echoes the raw error text.

**Paths over IPC.** The renderer only ever sees display strings for paths.
Native path bytes stay in Rust (spec section 6).

## Service interfaces

Traits exist only where tests need a fake, or to reserve a name (spec
section 9, abstraction budget). They are defined in
`src-tauri/src/domain/traits.rs`, except `JobProcessor`, which lives with
the runner in `services/jobs` because processors are handed the catalog
writer.

| Trait | Purpose | First adapter |
| --- | --- | --- |
| `CatalogRepository` | Source-root records (job rows live in `jobs::db`, the scan frontier in `inventory::db`; browse and open queries in `library` and `source_open`, on the read-only pool) | **0.3 (`SqliteCatalog`)** |
| `InventorySource` | Streamed, read-only directory listing | **0.5 (`LocalFsSource`)** |
| `VolumeInspector` | Free space, filesystem type, network or cloud-synced | **0.2 (`OsVolumeInspector`)** |
| `ResourceMonitor` | Device snapshot for the data-root volume | **0.4 (`OsResourceMonitor`, `SimulatedMonitor`)** |
| `ModelProvider` | Endpoint probe and model listing; capability tests (2.1) | **0.7 (`HttpModelProvider`)** |
| `EmbeddingBackend` | Text to vectors | **2.2 (`HashedBagOfWordsBackend`, the test backend)**; the provider-backed one in 2.3 |
| `SourceOpener` | OS-native open and reveal | **0.6 (`OsOpener` via `tauri-plugin-opener`)** |
| `JobProcessor` (`services/jobs`) | Runs the jobs of one kind; a `ProcessorFactory` builds one per open library | **1.1 (`ScanProcessor`)**; extraction in 1.3 |
| `DocumentProcessor` | Reserved name for Phase 1 extraction | Phase 1 |

`JobProcessor` arrived with the second job kind, as Phase 0 planned
([ADR 0009](adr/0009-jobs-and-extraction.md)). The scheduling policy stays
concrete: every kind obeys the same `ResourcePolicy`, so there is no
`SchedulingPolicy` trait.

`ModelProvider::test_connection` returns `impl Future + Send` instead of using
`async fn`, so callers can rely on `Send`. Traits are not yet required to be
object-safe; whether adapters are held as generics or `dyn` is decided when
the first adapter lands.

## Security boundary

- The only Tauri capability granted to the renderer is `core:default`. The
  dialog plugin is registered, but only Rust calls it (`pick_folder`), so the
  renderer has no dialog or filesystem permission. No shell, opener, or HTTP
  plugin is enabled.
- The renderer reaches native functionality only through Atlas commands, and
  the Rust side validates every ID and path.
- A strict Content Security Policy in `tauri.conf.json` allows only bundled
  scripts and IPC connections.
  - **No inline styles:** Tauri adds a nonce to `style-src`, so WebKit
    ignores its `'unsafe-inline'` and blocks every `style` attribute. jsdom
    applies no CSP, so `src/lib/csp.test.ts` checks that no component sets
    one, and the smoke checks that the page view's outline is drawn (Slice
    3.9: the DMG smoke found it blocked).
- Source files are never opened through string-built shell commands.
- Model requests leave only from Rust, to the resolved profile's own
  address, with its key as a header, through no redirect or proxy. Document
  text, questions and model output are data: the chat model has no tools,
  and excerpts sit in escaped, delimited blocks
  ([Ask Atlas](#ask-atlas)).

## Frontend

Svelte 5 with runes and Vite, without SvelteKit, because there is no SSR or
routing need. A small store selects the current view. At launch,
`src/lib/boot.ts` asks the core for the data-root status and routes:
not configured goes to Welcome and then the data-location screen, open goes
to Home, and missing, in use, incompatible or invalid goes to a recovery
screen. The theme is saved in the data root's `config/settings.json`;
browser storage is not used for Atlas preferences.

Main views: Home, Search, Library, Trails and Settings (`NAV_VIEWS` in
`src/lib/stores/view.svelte.ts`; Trails since Slice 3.4).

- **Search** (Slice 1.5, `views/Search.svelte`, `stores/search.svelte.ts`).
  Home's box and the Search view's own box submit to `search_documents`;
  nothing is searched while typing. Results show the title or file name,
  type, folder, where (page or chapter), the snippet, why it matched ("In
  the text", "In the name", both), "Some of your words" for any-words
  results, and "Also in N other places". Snippets are rendered from
  `SnippetSpan`s as text and `<mark>`, never as HTML. Empty states cover a
  punctuation-only query (not sent), nothing read yet, and no matches with
  how much has been read; while reading continues a note says so and the
  shown results refresh at most every 2 s.
- **Document details** (Slice 1.5, `components/DocumentPanel.svelte`,
  `stores/details.svelte.ts`). A modal side panel opened from a Library
  name or a result's Details: title, authors, language, pages (chapters for
  EPUBs), type, size, modified, folder, path, text status with its reason,
  and Also at with each copy's status and its own Open and Reveal. Escape or
  Close returns focus to what opened it; Tab stays inside. Since Slice 2.5,
  [Related](#related-documents) follows Also at.
- **Titles** (Slice 1.6). Results, the panel and duplicate groups show the
  title the core chose ([Metadata, duplicates and storage](#metadata-duplicates-and-storage)),
  with "Title from the first page" or "Title from the file name" when it
  isn't the file's own metadata, and the file name as a second line
  unless the title is the file name.
- **Library › Duplicates** (Slice 1.6, `components/DuplicatesList.svelte`,
  `stores/duplicates.svelte.ts`). A Files | Duplicates switch in the Library
  toolbar (a radio group). Duplicates lists groups of identical copies, most
  extra space first, with a summary ("2 groups · 9.4 MB in extra
  copies"), each copy's folder, path and status, and Details, Open and Show
  in folder per copy. Folder, Type and Include missing filter it; the text
  search, Text status and sorting are hidden. Atlas never deletes or moves
  a copy, and says so.
- **Settings › Storage** (Slice 1.6, `components/StoragePanel.svelte`).
  Text, Search index, Rest of the catalog and Caches, the indexing
  estimate as one sentence (always a range), and **Clean caches** with what
  it freed. "Measuring…" until the numbers arrive.
- **Shared pieces.** `stores/originals.svelte.ts` holds Open and Reveal with
  the cloud-download question and per-file problems, used by the Library,
  Search and the panel (`OriginalActions`, `OriginalNotice`); `TextBadge`
  is the Text status badge.

Accessibility baseline: landmarks and a skip link, `aria-current` on
navigation, focus moved to each view's heading, Cmd/Ctrl+1–5 view shortcuts,
visible focus rings, 44px major controls, `prefers-reduced-motion`, and
system light/dark via `prefers-color-scheme` with a `data-theme` override.

## Data root

Implemented in `src-tauri/src/services/workspace/`. Design rationale is in
[ADR 0002](adr/0002-data-root-creation-locking-and-compatibility.md).

```text
<root>/atlas.json             manifest: root ID, layout and schema versions (written last)
<root>/atlas.lock             exclusive OS lock while a process has the root open
                              (a held lock is retried for 0.5 s first, Slice 3.9)
<root>/config/settings.json   portable preferences (theme)
<root>/library/catalog.sqlite SQLite catalog, WAL mode (+ -wal, -shm)
<root>/library/knowledge.sqlite what the user creates: trails, bookmarks, findings, collections (Slice 3.2; ADR 0016)
<root>/backups/knowledge/      snapshots of the knowledge store, knowledge-<UTC time>.sqlite, the newest 14; never cleaned
<root>/backups/knowledge/staging/  a snapshot being written and checked; emptied on open
<root>/tmp/                   staging for atomic writes; emptied on open and by Clean caches
<root>/cache/                 rebuildable caches (empty so far); emptied by Clean caches
<root>/indexes/vectors/        the vector index per embedding model, <model>-<generation>.usearch (Slice 2.2); derived, rebuilt when needed
<root>/logs/                  the log, atlas-<UTC date>[-n].jsonl, at most 50 MiB (Slice 4.1; ADR 0021); never cleaned by Clean caches
<root>/exports/diagnostics/   diagnostics reports the user saved (Slice 4.1); exports/ is also the export dialog's first folder
<root>/content/ assets/ jobs/ models/   reserved, empty
```

- **Default location:** the OS per-user app-data folder. On macOS that is
  `~/Library/Application Support/com.altazlabs.atlas/Library`, on Windows
  `%LOCALAPPDATA%\Atlas\Library`, and on Linux `$XDG_DATA_HOME/atlas/library`.
- **Locator:** `locator.json` in the OS config folder records the root's path
  and ID. It is the one Atlas file outside the root. Debug builds honor
  `ATLAS_CONFIG_DIR` to redirect it for tests and smoke runs.
- **Inspecting** a location is read-only apart from a write probe. It reports
  free space, whether Atlas can write there, the volume kind, and what is
  already there: a new folder, an empty folder, an existing library, an
  interrupted setup, a folder with other files, or a file.
- **Creating** writes `.atlas-init`, takes the lock, creates the layout,
  catalog and settings, and writes `atlas.json` last and atomically. An
  interrupted creation is finished on the next attempt and keeps its ID.
- **Opening** never creates a root. It refuses a missing root, a damaged
  manifest, a newer layout or schema, a network or cloud-synced volume, and
  a root another process holds.
- **Catalog:** one writer thread owns the only read-write connection, with
  `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`, and a 5 second
  busy timeout. A new catalog gets 16 KiB pages (`NEW_CATALOG_PAGE_SIZE`,
  set before the switch to WAL fixes it; Slice 1.7), so the 2–4 KB
  passages share pages instead of each half-filling a 4 KiB one; a catalog
  created earlier keeps 4 KiB ([ADR 0010](adr/0010-full-text-search.md)).
  Migrations in `src-tauri/migrations/` run in order, each in
  its own transaction. The schema version is read before anything is
  written, so a newer catalog is refused untouched.
- **Data-root paths** must be absolute and valid Unicode. Source paths in
  later slices are stored as native bytes and are not restricted this way.

## Library folders (source roots)

Implemented in `src-tauri/src/services/sources/`; rationale in
[ADR 0003](adr/0003-source-root-identity-and-overlap.md).

- **Read-only.** The service only calls `metadata`, `read_dir` (dropped
  after the first call), `canonicalize`, and read-only directory opens for
  identity checks. Integration tests snapshot names, sizes and mtimes of a
  source tree before and after add, list and remove.
- **Storage.** `source_roots` (migration 0002) keeps the canonical path
  (`dunce::canonicalize`, so Windows paths keep their ordinary `C:\…` form
  rather than the verbatim `\\?\C:\…` one where that is safe; names that
  aren't valid Unicode stay verbatim, so path comparisons go through
  `native_path::comparable`) as platform-tagged native bytes (`infrastructure/native_path.rs`) plus a
  lossy display string. There is no stored status column: availability is
  probed live each time the list is shown, with a 2 second budget per call
  so a hung network mount cannot freeze the UI.
- **Rules on add.** The folder must exist and be readable. The data root and
  anything inside it are refused; a folder containing the data root is
  allowed and flagged (`containsDataRoot`), and inventory (0.5) will skip the
  data root's subtree. Duplicates and folders inside or containing an
  existing root are refused, comparing filesystem identity with `same-file`
  so symlinks and case-insensitive spellings are caught.
- **Existing overlaps** (possible only in imported catalogs) resolve
  deterministically: the outermost root owns the subtree, and inner roots
  report `nestedInRootId`. **Slice 0.5 must scan only roots without
  `nestedInRootId`**, attributing every file to the outermost root.
- **Picker.** `pick_and_add_source_roots` runs the native multi-folder
  picker in Rust, so native paths never cross IPC and folder names that are
  not valid Unicode still work. `add_source_root(path)` accepts UTF-8 paths.
- **macOS privacy.** `src-tauri/Info.plist` supplies the usage strings macOS
  shows before Atlas first reads Documents, Desktop, Downloads, removable
  or network volumes.

## Device check and resource policy

Rationale in [ADR 0004](adr/0004-resource-policy.md).

- **Probe** (`infrastructure/system_probe.rs`). Memory comes from `sysinfo`,
  cores from `available_parallelism`, the data drive from
  `OsVolumeInspector`, and power from `starship-battery`. When no batteries
  are found the computer is treated as a desktop on AC; when the battery
  probe fails, power is unknown. Memory pressure comes from the macOS
  `kern.memorystatus_vm_pressure_level` sysctl or Linux PSI. Thermal state
  comes from macOS `NSProcessInfo.thermalState`. Windows memory pressure and
  thermal state, and Linux thermal state, are reported as unknown. Each
  sub-probe fails independently, and a sample takes well under 500 ms.
- **Policy** (`services/scheduler/policy.rs`) uses a first-match table. It
  pauses for an unreachable data drive, low space, critical pressure, or
  battery at or below 20% (resuming at 25% or on AC). It runs slowly on
  battery, with unknown power, or under elevated pressure, and runs normally
  otherwise. Rates are 500 and 100 metadata operations per second, chosen
  from the Slice 0.8 measurements ([benchmarks](benchmarks.md)).
- **Assessment** (`services/system/assessment.rs`) maps memory to library and
  local-AI fitness at 8, 16 and 32 GiB, plus note codes. The UI owns the
  wording, in `src/lib/deviceCopy.ts`.
- **Sampling** is on demand: `get_device_check`, the Device check screen, and
  Settings every 30 seconds while it is open. The 0.5 scan loop samples
  before each batch.
- **Simulation:** debug builds honor `ATLAS_SIMULATE_POWER` and
  `ATLAS_SIMULATE_FREE_BYTES`, exposed as `just dev-simulate`.

## Jobs

Implemented in `src-tauri/src/services/jobs/` (Slice 1.1); rationale in
[ADR 0009](adr/0009-jobs-and-extraction.md).

- **Schema** (migration 0007). `jobs` is Phase 0's `scan_sessions` renamed,
  plus a `kind`: `scan` now, `extract` from Slice 1.3. The state machine
  and its columns are Phase 0's (phase0-spec §7.3). There is no CHECK on
  `kind`. Rust's `JobKind` rejects an unknown kind when a row is read, and
  a CHECK would need a table rebuild to extend, which would cascade under
  foreign keys (`src-tauri/migrations/README.md`).
- **Runners.** Each open library runs one thread per registered kind
  (`atlas-job-scan`), owned by `OpenRoot` and stopped before the catalog
  closes. Closing signals every runner before joining any. A runner picks
  its kind's running job, then a policy-paused one, then a queued one (by
  priority and age), and hands it to the kind's `JobProcessor`. It sleeps
  when nothing is runnable (a wake cuts the sleep short) and backs off on
  database errors.
- **Processors.** `Workspace::with_job_processor(kind, factory)` registers
  a kind, and the factory builds a processor for each library opened, on
  the runner's own thread. `with_inventory` registers the scan processor.
- **Control.** `pause_job`, `resume_job` and `cancel_job` take any job ID,
  with Phase 0's transitions and error codes. Each is a conditional state
  change on the writer thread, then a wake of every runner. Processors
  guard their writes with `state = 'running'` on the same writer, so
  nothing lands after a pause.
- **Recovery.** On open, interrupted jobs of every kind are queued again; a
  user's pause is kept. A folder that has never been scanned gets one scan.
- **Status.** The `inventory-status` overview, the per-folder lines and the
  Library's "scanning" flag count scan jobs only. Extraction reports its
  own status from Slice 1.3.

## Inventory

Implemented in `src-tauri/src/services/inventory/`; rationale in
[ADR 0005](adr/0005-inventory-durability.md).

- **Schema** (migrations 0003 and 0007). A scan is a `jobs` row of kind
  `scan` (the table was `scan_sessions` until 0007). `scan_directories` is
  the persisted frontier of folders still to list. `file_instances` holds one row per PDF or EPUB, keyed on
  `(source_root_id, rel_path)`, with the path relative to the folder and
  stored as native bytes. Each row also has size, whole-second mtime,
  `availability` (available or missing), and `storage` (local or
  cloud_only).
- **Scan processor.** `ScanProcessor` (`inventory/scan.rs`, Phase 0's
  worker) runs on the scan runner (see [Jobs](#jobs)). It lists folders in
  batches of up to 100, one transaction per batch. A batch commits only while its
  session is still running, so a pause or cancel leaves no late writes.
  Every 2 seconds it re-checks the source folder and the resource policy,
  and it paces itself with a token
  bucket. A folder interrupted part-way is listed again from its first
  entry on resume; entries this scan already wrote unchanged are left
  untouched by the upsert and cost a twentieth of an operation
  (`REPLAY_COST_DIVISOR`, Slice 1.7), so the next new file arrives in
  about a second even 9,000 entries into a 20,000-file folder (P1-20). Classification is by extension only, and file contents are never
  opened.
- **Triggers.** A session starts when a folder is added, on Rescan, or when
  a library opens with interrupted work or a never-scanned folder. User
  pauses persist across restarts.
- **Missing files.** A file is marked missing only when its folder was
  listed successfully in a session and the file wasn't found. A folder
  that can't be read protects its whole subtree.
- **Links and long paths.** Symlinks, and on Windows junctions (which std
  reports as symlinks), are skipped and counted, never followed. Paths are
  stored as native bytes with no length limit; on Windows, std adds the
  `\\?\` prefix to paths past 260 characters, so they are cataloged and
  read like any other (P1-21, Slice 1.7). Opening one goes through the
  Windows shell, which isn't tested on paths that long.
- **Cloud files.** iCloud dataless files, legacy `.icloud` placeholders,
  and Windows cloud placeholders are cataloged as `cloud_only` without
  being downloaded. Listing a dataless folder has a 15 second timeout.
  Once a file is downloaded, the next Rescan of its folder reads it, though
  a download keeps its size and time (Slice 3.1).
- **Progress.** The `inventory-status` Tauri event carries an
  `InventoryOverview`, at most 4 per second plus on every state change.
  The UI shows counts only, never totals or percentages. The counts come
  from a covering index (migration 0006), so building an overview stays
  cheap as the catalog grows. `get_inventory_status` reads on the
  read-only pool, so it never waits behind batch commits
  ([benchmarks](benchmarks.md)).
- **Fixtures.** `just fixtures <dir> [files] [flat]` writes a synthetic
  tree outside the repository (`fixtures/README.md`). `just dev-unthrottled`
  removes the rate limit in debug builds.

## Library, search and opening

Implemented in `services/library/` and `services/source_open/`; rationale
in [ADR 0006](adr/0006-search-keys-and-safe-opening.md).

- **Reads.** `CatalogReader` keeps 2 read-only connections, so UI queries
  never wait for inventory writes. `OpenRoot` drops it after the job
  runners and before the writer.
- **Search.** `search_key` (the root-relative path) and `name_key` are
  folded: NFD, combining marks removed, lowercase, NFC. Each search term
  becomes an escaped `LIKE` on `search_key`, and terms are ANDed. Filters
  cover folder, type, and include-missing. Sorting is by name or modified
  date, and pages are offset-based with a size of 1–200. Measured p95 is
  33 ms on 30,000 files in a debug build.
- **Status per row.** Each file is available, missing, or `unreachable`
  (its folder isn't connected, from a probe cached for 5 seconds), plus
  local or cloud-only.
- **Open and reveal.** These work by file ID. The reachable-root, exists,
  regular-file, inside-root, and dataless checks all run in Rust, and the
  OS opener receives the native path. A cloud-only file needs
  `allowDownload: true`. For a legacy iCloud placeholder, Atlas reveals the
  placeholder instead of opening it.

## AI service connections

Implemented in `services/providers/`, `infrastructure/http_probe.rs`, and
`infrastructure/keychain.rs`; rationale in
[ADR 0007](adr/0007-provider-connections-and-secrets.md) and
[ADR 0012](adr/0012-model-roles-and-capability-tests.md).

- **Profiles** (migration 0005) store kind, label, base URL, an insecure-HTTP
  acknowledgement, and *where* the key is kept. The key itself is never in
  the catalog.
- **Keys** go to the OS keychain under service `com.altazlabs.atlas`, or
  `….debug` in debug builds, with account `provider:<id>`. If the keychain
  is unavailable, they are kept in memory for the session only. The
  keychain is opened on first use (`LazyKeychain`, at most 5 s), never at
  startup. A redacting `Secret` type keeps keys out of `Debug` output.
- **Address rules:** http(s) only, no credentials, query, or fragment; plain
  HTTP silently to loopback only, otherwise per-profile acknowledgement.
- **Connection tests** are bodiless GETs of the model list, with no
  redirects, no proxies, bounded time and size, and honest outcomes. A
  listing is never reported as chat or embedding support.
- **Capability tests** (Slice 2.1) send one fixed POST to one model:
  `{"input": ["Atlas capability test"]}` to `{base}/embeddings` or Ollama's
  `/api/embed` (the answer's vector length is the model's dimensions), or
  "Reply with the word OK." to `{base}/chat/completions` or `/api/chat`,
  streamed (SSE or NDJSON) and passing on the first token, a reasoning
  model's thought included. Two minutes at most, since a local runtime may
  load the model first; the reply itself is never kept. Failures are
  classified from the status and the service's error message: model not
  found, wrong role, authentication, protocol mismatch, network, timeout.
- **Roles** (migration 0011, `model_roles`): a profile and model for
  `chat` and for `embedding`, set only with a passing capability test of
  that model from the last 24 hours (`ModelNotTested` otherwise), with the
  embedding model's dimensions; deleting the profile clears its roles.
- **Recommendations** come from `services/providers/recommendations.json`,
  compiled in: per role, a recommended model plus faster or
  higher-quality alternatives, filtered by the computer's memory, with
  steps for oMLX, Ollama and LM Studio and whether each runtime is known to
  serve it. Model ids the user's services already listed are matched (case
  and separators ignored) so an installed copy is offered directly. No
  network, and Atlas never downloads a model.
- **Privacy label** (`url_policy::location`): This computer (loopback),
  Another computer (private, link-local and CGNAT addresses, `.local`,
  `.lan`, `.home.arpa`, `.internal` and single-label names), or Internet
  service, from the address alone without DNS.
- **Detection** probes loopback well-known ports only, and only when the
  user asks.

The webview also creates its own website-data and cache folders, on macOS
`~/Library/WebKit/<bundle>` and `~/Library/Caches/<bundle>`. These are
platform-required and contain no Atlas data.

## Extraction worker

Implemented in `src-tauri/src/services/extraction/` (the worker in Slice
1.2, extraction in 1.3); rationale in [ADR 0009](adr/0009-jobs-and-extraction.md) and
[ADR 0008](adr/0008-extraction-and-vector-engines.md).

- **Process.** `atlas --extract-worker` is checked first in `main.rs`, so
  the worker starts no window, plugin or workspace. It reaches `hello` in
  about 16 ms on the M5 Pro ([benchmarks](benchmarks.md#extraction-worker-start)),
  well under the 300 ms at which the spec would switch to a separate
  sidecar binary, so it stays a mode of the app executable.
- **Protocol.** JSON lines over stdin and stdout
  (`extraction/protocol.rs`). The first line is
  `{"type":"hello","protocol":2,"extractorVersion":…,"pdfiumVersion":"chromium/8125","selfTest":{"pages":2,"chars":…}}`,
  or `{"type":"unavailable","reason":"not_found|load_failed|self_test_failed"}`
  followed by exit code 3. Then one request at a time: `hash {path}` →
  `hashed {hash, size, mtimeSecs}` or `hashFailed {reason}`; `extract
  {path, kind}` → `page` frames, then `done {summary}` or `failed {outcome,
  reason}`; from protocol 2 (Slice 3.7), `render {path, page, width,
  find?}` → `rendered {page, pageCount, width, height, png, rects, found}`
  or `renderFailed {reason}` ([Page view](#page-view)). Paths are the
  catalog's platform-tagged bytes, hex-encoded.
  Reasons are fixed codes, never paths; library messages go to stderr.
- **Engines** (`extraction/engine/`, worker only). A signature sniff (`%PDF-`,
  or a zip for EPUB); streaming BLAKE3; PDFium with one passage per page;
  for EPUBs a zip pre-scan before rbook reads anything (DRM, zip-slip,
  32 MiB per entry, 256 MiB per archive), then rbook and html2text in spine
  order, split into pieces of at most 4,000 characters, leaving out
  Project Gutenberg's header and licence (`engine/gutenberg.rs`, reader
  version 3, Slice 3.1; counted in `documents.boilerplate_chars`); and the
  unreadable-text rule (half the pages under 20 visible characters, or 10%
  control, private-use or replacement characters → `needs_ocr`, no text
  stored; since Slice 4.1 such a PDF keeps its `page_count` and
  `text_pages`, the pages with at least 20 visible characters under a
  tenth garbled, sent in the `failed` frame's two optional fields). Files are opened read-only, never recalling cloud content
  (`FILE_FLAG_OPEN_NO_RECALL` on Windows; dataless files refused).
- **Output hygiene.** Before PDFium loads, the worker keeps a duplicate of
  stdout for the protocol and points descriptor 1 (and on Windows the
  standard output handle and the C runtime's descriptor 1) at stderr, so
  anything a library prints can't corrupt the stream. A test writes to
  descriptor 1 on purpose to prove it.
- **Self-test.** PDFium opens a two-page PDF compiled into the binary
  (`extraction/selftest.pdf`, written by the fixture generator) and must
  find its marker phrase before the worker says hello. This proves the
  bundled, signed library loads and works.
- **Finding PDFium** (`extraction/locate.rs`). Debug builds honour
  `ATLAS_PDFIUM_LIBRARY` as the only candidate. Otherwise: macOS
  `Contents/Frameworks/libpdfium.dylib`; Windows and Linux the Tauri
  resource folder (next to the executable on Windows and in Cargo's
  `target/`, `../lib/Atlas`, `$APPDIR/usr/lib/Atlas` or `/usr/lib/Atlas` on
  Linux), mirroring `tauri_utils::platform::resource_dir`. Debug builds fall
  back to `src-tauri/vendor/pdfium/<platform>/`.
- **The main process never loads PDFium.** `pdfium-render` binds it at run
  time (`dlopen`/`LoadLibrary`), and only the worker calls it.
- **Lifetime** (`extraction/lifetime.rs`, Slice 1.7, P1-22). Quitting stops
  the job, and the supervisor kills the worker before the catalog closes.
  If the app is killed or crashes instead, a watchdog thread in the worker
  ends it as soon as its parent (`ATLAS_WORKER_PARENT`) is gone: on Unix
  when the worker is reparented, checked every 250 ms; on Windows when a
  wait on the parent's handle returns. It exits without running exit
  handlers, since PDFium may be mid-call. On Windows each worker is also
  in a kill-on-close Job Object the app holds open, so the kernel ends it
  with the app. One extraction worker per library (`ATLAS_EXTRACT_WORKERS`
  is not built; [ADR 0009](adr/0009-jobs-and-extraction.md)); the page
  view has a second, for rendering only (Slice 3.7).

## Extraction

The job side, in the app process (Slice 1.3, ADR 0009).

- **Schema** (migrations 0008 and 0010). `documents` is content, keyed by BLAKE3:
  state (`done`, `needs_ocr`, `encrypted`, `malformed`, `unsupported`,
  `too_large`, `worker_crashed`, …), reason code, extractor version, page
  and character counts, title, authors, language, a first-page title
  (`content_title`) and a `refresh` flag. `passages` holds its
  text. `file_instances` gains `extract_state` (`pending`, `linked`,
  `skipped_cloud_only`, `unreadable`, `hash_failed`), the hashed size and
  mtime, `document_id` and `extract_attempts`. A rescan that sees a new
  size or mtime sets the file back to `pending`, as it does a
  `skipped_cloud_only` file it now finds stored locally (Slice 3.1).
- **Supervisor** (`extraction/supervisor.rs`) implements `DocumentProcessor`
  over the worker: a 30 s inactivity timeout per request, a memory cap of
  `min(1 GiB, 15% of RAM)` polled every 250 ms, kill and restart, and a kill
  on drop.
- **Job** (`extraction/job.rs`, kind `extract`). One per source root,
  queued when a scan finishes with pending files, when a library opens, or
  by **Read text now** (`start_extraction`). Per file: cloud-only files are
  skipped unopened; the worker hashes the file; a document already read by
  the current reader for its type is linked (copies and moves); anything
  else is extracted and committed with its passages in one transaction,
  guarded by the job still running. Two crashes mark a file
  `worker_crashed`; ten restarts back the job off. After the waiting files,
  the job re-reads documents marked `refresh` (below). Before it
  completes it reads any file added after its first pass: a scan that
  finishes while a job runs queues no other (Slice 3.4, found by P3-13).
- **Reader versions and re-reading** (Slice 1.6, [ADR 0011](adr/0011-re-reading-without-losing-search.md)).
  Each format has its own version (`READER_VERSION_PDF`,
  `READER_VERSION_EPUB`, both 2: first-page titles; EPUB text without
  Markdown marks). On open, a document read by an older reader either moves
  to the current version (no text, or a PDF whose metadata already gives a
  usable title: its output would not change) or is marked `refresh`. A
  refreshing document's files stay linked and "Text ready"; one local copy
  is read again and the new text replaces the old in one transaction, so
  search never loses it. A refresh the worker crashes on twice keeps the
  old text. The status line says "Updating text · N files to go".
- **Policy.** The scan's governor and battery hysteresis, as a duty cycle:
  50% busy on Run, 20% when slowed, paused below 20% battery until 25%.
  Each file owes a rest in proportion to its time; owed rests are taken
  once they reach 100 ms (`REST_QUANTUM`), since a sleep after every
  millisecond-long file costs more than it rests (Slice 1.7).
- **Status.** `extraction-status` (an `ExtractionOverview`: the active job
  with the file being read, counts by text status, per-folder counts) at
  most four times a second plus every change; `get_extraction_status`
  reads it on the read-only pool. The Library shows a Text column and
  filter; the status bar a second line with its own Pause, Resume and
  Cancel.


## Metadata, duplicates and storage

Slice 1.6, in `src-tauri/src/services/documents/` and `services/storage/`.

- **Titles** (`documents/metadata.rs`, P1-18). Stored metadata stays as the
  file had it; cleaning happens when the catalog is read, so new rules
  never need a re-read. `clean_title` drops placeholders ("Untitled",
  "Document3", "Slide 1"), file names and paths, and the "Microsoft Word -
  … .docx" wrapping; `clean_authors` splits on `;` (not commas: "Lin,
  Ada") and drops account names ("Administrator"). The displayed title is
  the cleaned metadata title, else a PDF's first-page title, else the
  tidied file name (`TitleSource`).
- **First-page titles** (`extraction/engine/title.rs`, in the worker). Page
  1's characters with their size, position and angle are grouped into
  runs; rotated runs (arXiv's side stamp) are dropped; the largest text
  must be at least 1.25× the body size, and up to three consecutive lines
  of it (at most 25 words) form the title. All six arXiv papers in the
  spike corpus get their real title this way.
- **Duplicates** (`documents/duplicates.rs`, `list_duplicates`). A group is
  a document with at least two available copies inside the folder and type
  filters; missing copies are listed on request but never counted. Groups
  are ordered by extra space ((copies − 1) × size), paged by 20, with one
  query for the groups and totals (index `file_instances (document_id,
  availability)`, migration 0010) and one per group for its copies.
- **Storage** (`services/storage/`, `get_storage_usage`, `clean_caches`).
  The catalog's size is its files (`catalog.sqlite`, `-wal`, `-shm`); the
  text and index shares come from SQLite's `dbstat` on a read-only
  connection of their own, outside the workspace lock; caches are `cache/`
  and `tmp/`, walked without following links. The estimate appears after
  50 documents are read: the text and index bytes per read document times
  the files still waiting, ±40%. Clean caches empties `cache/` and `tmp/`
  (keeping the library's own folders, removing links rather than their
  targets) while holding the workspace, so nothing is staging in `tmp/`.
  Since Slice 3.2 Storage also shows "Your trails and notes" (the knowledge
  store with its log) and "Snapshots (N)", apart from the catalog, and a
  sentence when the store can't be used or the last snapshot didn't happen
  (`get_knowledge_status`). Clean caches never touches either. Since Slice
  4.1 "The log" (`logs/`) is measured apart too, and Clean caches leaves
  it.

## Search

Implemented in `src-tauri/src/services/search/` (Slice 1.4; meaning search
Slice 2.4); rationale in [ADR 0010](adr/0010-full-text-search.md) and its
hybrid-search amendment.

- **Hybrid search** (`search/hybrid.rs`, `search::run_hybrid`, Slice 2.4):
  - The query is embedded (`embedding/query.rs`: the serving model, its
    prefix, a 256-entry cache, a 1.5 s wait) while the text pass runs.
  - The 200 nearest chunks are filtered through `embedding/chunk_map.rs`
    and kept above the model's similarity floor.
  - Reciprocal rank fusion (k = 60) runs in two tiers: every-word matches
    first. The best meaning match counts twice, so it leads the second
    tier even without matching words (Slice 3.1.1).
  - Reasons `semantic` and `text_semantic`; `SearchPage.semantic` says
    whether meaning search took part.
  - The search command runs off the workspace lock.

- **Index** (migration 0009). An external-content FTS5 table over
  `passages`, `unicode61 remove_diacritics 2`, `detail='full'`, kept in step
  by triggers; built once from the stored text at upgrade, merged after
  each reading pass.
- **Query builder** (`search/query.rs`). Words and quoted phrases, each
  quoted for FTS5, so nothing typed is an operator; a trailing `*` is a
  prefix. All words first; when fewer than a page of documents match, an
  any-words form without common words fills the page (`allWords: false`).
- **Results** (`search/mod.rs`, `search_documents`). The best 500 passages
  by bm25, grouped by document; folder, type and missing filters; file-name
  matches merged (`why: both` ranks a little higher, name-only after);
  paging over documents; one file per result with its other copies
  counted. Runs on the read-only pool (4 connections).
- **Snippets** (`search/snippet.rs`). FTS5 `snippet()` with private-use
  markers, turned into `SnippetSpan { text, hit }`; stored text never
  contains the markers.
- **Evaluation.** `fixtures/evaluation/questions.json` (40 questions,
  committed before search was built); `tests/search_eval.rs` requires a
  relevant document in the top 5 for 80% (all 40 pass). `just eval` asks a
  question set against any library, opening its catalog read-only.
- **Measured** on 30,000 files with 622 million characters: p95 152 ms
  while reading ([benchmarks](benchmarks.md#search-slice-14)).
## Embeddings and the vector index

Implemented in `src-tauri/src/services/embedding/` (Slice 2.2); rationale
in [ADR 0013](adr/0013-embeddings-durable-index-disposable.md).

- **Rows** (migration 0012, `embedding/db.rs`):
  - `embedding_models`: a model name at a dimension count and chunker
    version; it also records its saved index file.
  - `chunks`: AUTOINCREMENT ids, which are the index's keys, with character
    offsets into a passage. Shared by every model.
  - `embeddings`: f16 BLOBs keyed by model and the BLAKE3 of the chunk's
    text, so identical text is embedded once.
  - `document_embeddings`: per document and model, with a per-model `seq`.
    No row means the document is waiting.
  - `vector_tombstones`: sequenced, filled by a trigger when chunks are
    deleted.
  - `commit_document` clears a re-read document's embedding state.
- **Chunks** (`embedding/chunk.rs`): at most 1,200 characters with about
  150 of overlap, cut at paragraphs, then sentences, then spaces, never
  across passages.
- **Index** (`embedding/index.rs`, `VectorIndex`): usearch HNSW, cosine,
  f16, connectivity 16, expansion 128.
  - Searches and saves share a read lock; adds and removes take the write
    lock.
  - Saves are atomic and generation-named: staged in
    `indexes/vectors/staging/`, renamed into `indexes/vectors/`, hashed, then
    recorded in the catalog. A file with over 20% removed slots is rebuilt
    when opened.
  - `open_index` loads a file only if its BLAKE3 matches the catalog, then
    reconciles it: it adds documents after its `seq` and removes
    tombstones. Otherwise it rebuilds from the catalog.
  - On Windows, a path outside ASCII goes through memory buffers instead of
    usearch's own file I/O.
- **Test backend** (`embedding/fake.rs`): `HashedBagOfWordsBackend`, which
  hashes words into a signed bucket. It is deterministic, and shared words
  give nearby vectors.
- **Measured** at a million 1,024-dimension synthetic vectors: recall@10
  0.960, p95 0.82 ms, open to first answer 1.53 s, rebuild 54 s
  ([benchmarks](benchmarks.md#vector-index-slice-22)).
- **The embedding job** (Slice 2.3, `embedding/job.rs`; rationale in
  [ADR 0014](adr/0014-the-embedding-job.md)):
  - `JobKind::Embed`, one job per folder. It sends only chunks whose text
    has no vector, through the embedding role's service
    (`embedding/client.rs`, OpenAI-compatible or Ollama).
  - It writes each document in one transaction, guarded by a fingerprint
    of its text, then adds its chunks to the in-memory index held by the
    library's `VectorRegistry` (`embedding/registry.rs`).
  - It pauses on battery. It backs off on network, timeout, 429 and 5xx
    errors, then fails for a retry; a refused key, a missing model or
    changed dimensions fail it for good.
  - A model change (`embedding/models.rs`) builds a new index while the
    old one serves, validates it, then retires the old one.
  - Jobs are queued when a folder's text is read, when the library opens,
    when the role or a service changes, when the privacy setting allows
    it, and on "Index meaning now" (`embedding/triggers.rs`).
  - Status: `embedding/status.rs`, the `embedding-status` event. A
    folder whose latest job stopped for good carries its code
    (`RootEmbeddingSummary.failedBecause`, Slice 4.1.1), so the status
    line says why and offers Open Settings or Index meaning now, where
    it used to say only "Meaning waiting".
- **Old catalogs** (`catalog/convert.rs`): a catalog with 4 KiB pages,
  an embedding role and no vectors is rewritten with 16 KiB pages
  (`VACUUM INTO`, then a rename) when its library opens.
- **Before an embed job finishes** (Slice 4.1) it looks once more from the
  start if a document behind it was read again and never tried by the
  model, so a re-read during the job isn't left waiting until the next
  launch.

## Related documents

Implemented in `src-tauri/src/services/documents/related.rs` (Slice 2.5);
rationale in the related-documents amendment to
[ADR 0013](adr/0013-embeddings-durable-index-disposable.md).

- **`get_related_documents(documentId, limit)`** returns a `RelatedPage`:
  a status and up to ten `RelatedHit`s, each a document with an available
  file and the location of its best matching chunk. It takes the library's
  handles under the workspace lock and runs off it, as search does.
- **How:** the mean of up to 64 of the document's stored chunk vectors,
  re-normalised, is searched against the index. The document's own chunks
  are filtered out inside the index, and chunks under the model's floor
  are dropped. The rest are grouped by document, widening from 60 to 200
  neighbours when fewer than ten documents are left.
- **Never related:** the document itself (its copies are the same
  document), documents sharing 95% of their distinct chunk texts with it
  (the same text in other bytes), and documents with no available file.
- **Nothing is sent to a model:** it works while the AI service is down.
- **Cache:** 64 documents' candidates in the library's `VectorRegistry`,
  keyed by model, index generation and size; files are looked up afresh.
- **Frontend:** `RelatedList.svelte` in the document panel.
  - Details shows a related document in the same panel, with "Back to
    ‹title›" (`details.show`, `details.back`).
  - More like this on a search result opens the panel and moves focus to
    Related.
- **Measured:** p95 2.0 ms at 1,000 documents and 3.1 ms at 30,000
  ([benchmarks](benchmarks.md#related-documents-slice-25)).

## Ask Atlas

Implemented in `src-tauri/src/services/answers/` (Slice 2.6, the engine)
and `services/providers/privacy.rs`. The screen (Slice 2.7) is in
`src/lib/stores/answer.svelte.ts` and the components named below.
Rationale in [ADR 0015](adr/0015-ask-atlas-answers-citations-injection.md).

- **`ask_atlas({question, sourceRootId?, fileType?})`** returns an
  `AskReply`:
  - `started`: an answer id, the numbered `sources` (Sources used: exactly
    what is sent), `weak`, and the meaning-search status; `answer-stream`
    events follow;
  - `nothing_found`: the model wasn't asked;
  - `needs_disclosure`: an internet service not agreed to yet, with the
    number of excerpts and bytes; nothing was sent.
  - Errors: `chat_model_needed`, `external_ai_disabled`, `invalid_question`
    (1–2,000 characters).
- **Retrieve** (`retrieve.rs`, `search::retrieve`): hybrid search's best 8
  documents.
  - Excerpts come from the best meaning chunk with its neighbours, a
    second meaning place that doesn't touch or repeat it, and the text
    around the first word match.
  - About 24,000 characters in all, filled round-robin, at most 2,000 each.
  - Project Gutenberg's header and licence are cut out.
  - `weak`: fewer than 3 sources, or the best meaning match under the
    floor plus the model's weak margin, from the recommendation catalog
    (`weakMargin`; 0.20, tuned with Qwen3-Embedding-0.6B, and the default
    for models the catalog doesn't know; Slice 3.1).
- **Prompt** (`prompt.rs`): a fixed system message, then one user message
  of `<source id="n" title="…" location="…">` blocks and the
  `<question>`. Every `<` and `>` inside them is escaped.
- **Stream** (`stream.rs`): OpenAI-compatible SSE or Ollama NDJSON from the
  chat role's service. 120 s to the first piece, 60 s of silence, 1 MiB of
  answer. Thinking is asked to be off: Ollama's `think: false`, and
  `chat_template_kwargs.enable_thinking: false` for a service that isn't
  on the internet. Thinking that comes anyway is dropped and reported
  once. An answer with no text ends in `error` `no_answer`. Stop or a new
  question aborts the request.
- **Cite** (`cite.rs`): `[n]` is checked as it streams. Fragments are held
  until they resolve, invented numbers are removed and counted, and
  `citation` events mark first uses.
- **`answer-stream`**: `{answerId, step}`, where `step` is `thinking`,
  `delta` (at most every 50 ms), `citation`, `done {cited, dropped}`,
  `stopped`, or `error {code, message, partial, cited, dropped}`.
  `cancel_answer(answerId)` stops one.
- **One answer per library** (`AnswerRegistry`, the first field of
  `OpenRoot`, so it stops first when the library closes). It is kept only
  as a step of the user's trail, while recording is on
  ([Recording trails](#recording-trails), ADR 0017).
- **Privacy** (`config/settings.json`):
  - `allowRemoteEmbedding`, `disableExternalAi`, and the `acknowledged`
    internet services (profile, address, label, time).
  - `privacy::allows` decides for every request to an internet service:
    Disable all external AI refuses everything (Ask, embedding, query
    embedding, connection and model tests: `external_ai_disabled`);
    embedding needs the opt-in; Ask needs the agreement, which a changed
    address asks for again.
  - Services on this computer or another one are never restricted.
  - The Privacy panel (`PrivacyPanel.svelte`) has both checkboxes and the
    agreed services, each with Forget.
- **Measured** with `just eval-ask` against real models, run by the owner
  ([benchmarks](benchmarks.md#ask-atlas-slice-26)).
- **The screen** (Slice 2.7):
  - **Mode:** Search's "Find sources | Ask Atlas" radio group
    (`search.mode`). Filter changes in Ask mode don't search; Find sources
    searches again on return.
  - **`answer.svelte.ts`:**
    - It subscribes to `answer-stream` at boot and holds one answer: phase,
      sources, text, cited, dropped, and the failure or disclosure.
    - Steps that arrive before `ask_atlas` returns are kept by id and
      applied once the reply names it.
    - A library change resets it, cancelling a live answer.
  - **`AskForm.svelte`:** a textarea (Enter asks, Shift+Enter starts a new
    line), Ask or Stop, and a counter near 2,000 characters.
  - **`AnswerView.svelte`:**
    - status lines (finding, waiting, Thinking…, stopped);
    - the answer from `answerText.ts` pieces, with chips;
    - the weak, dropped and meaning notes;
    - failures, with Try again or a way to Settings;
    - a polite announcer.
  - **`SourcesUsed.svelte`** and **`DisclosureDialog.svelte`**.
  - **The details panel:** `details.open(fileId, trigger, { cited })`
    shows "Cited passage" first.
- **Smoke** (Windows and Linux, `e2e/smoke.mjs`):
  - It starts `e2e/fake-ai.mjs`, a scripted OpenAI-compatible service on
    127.0.0.1 (models, hashed bag-of-words embeddings, a canned answer
    citing `[1]`), and adds it in Settings as a user would.
  - It then chooses both models, waits for meaning, finds a short
    lighthouse book by a question in other words, and asks Atlas. The chip
    opens the cited passage.
  - `just fake-ai` runs the service by hand.

## Knowledge store

Implemented in `src-tauri/src/services/knowledge/` (Slice 3.2). Rationale
in [ADR 0016](adr/0016-the-knowledge-store.md); schema in
`src-tauri/migrations/knowledge/` (phase3-spec §6.1).

- **Opening** (`store.rs`) happens after the catalog and the directory
  repair, with the library's lock held. `KnowledgeStore::open` never fails:
  - It opens a writer thread (`CatalogWriter::open_named`,
    `atlas-knowledge-writer`) and reads `PRAGMA user_version` before
    writing anything.
  - A newer store is `incompatible`. A file that isn't SQLite, or is
    someone else's SQLite file, is `damaged`. Either way the file is left
    byte for byte, and `call`, `handle` and `read` return
    `knowledge_unavailable`.
  - Otherwise it switches to WAL, migrates, and opens two read-only
    connections. Version 2 (Slice 3.8, `0002_updated.sql`) adds
    `updated_at` to trails and steps, so an import can keep the newer copy.

  The library opens whatever the store's state. `OpenRoot.knowledge` is
  declared before `catalog`, so the store closes first.
- **Change tracking:** triggers on every user table bump `meta.changes`.
- **Snapshots** (`snapshot.rs`):
  - **When:** at open, and hourly on the store's own thread, if the store
    changed since the last snapshot and the newest is at least 24 hours
    old.
  - **How:** `VACUUM INTO` the staging folder on the writer thread,
    `integrity_check`, an atomic rename into `backups/knowledge/`.
  - **Kept:** 14, pruned only after a new one passed its check.
  - **Low space:** no snapshot, `low_space` in the status, nothing
    deleted.
  - **Time** comes from the workspace's `Clock` (`Workspace::with_clock`;
    tests use `TestClock`).
- **Anchors** (`anchor.rs`): `build(conn, file_id, place)` makes an anchor
  from the catalog. `resolve(conn, anchor)` gives `found`,
  `found_elsewhere`, `no_file` or `unknown` with the file and place to
  open, and never writes. The order of lookups is in ADR 0016.
- **`get_knowledge_status`** gives the state, the size with the log, the
  counts of trails, steps, bookmarks, findings and collections, and the
  snapshots' count, size, newest time and last outcome.
- **Restoring a snapshot by hand** (Phase 3 has no in-app restore):
  1. Quit Atlas.
  2. In the library folder, copy the snapshot you want from
     `backups/knowledge/` over `library/knowledge.sqlite`.
  3. Delete `library/knowledge.sqlite-wal` and `library/knowledge.sqlite-shm`
     if they are there.
  4. Open Atlas.

  The snapshot is a complete store. Anything saved after it is replaced.

## Recording trails

Implemented in `src-tauri/src/services/knowledge/{record,trails,meta}.rs`
and `src-tauri/src/commands/trails.rs` (Slice 3.3). Rationale, and the
privacy rules, in [ADR 0017](adr/0017-trails-and-privacy.md).

- **The `Recorder`** (`OpenRoot.recorder`, one per library) writes steps
  from Rust's own state. The renderer sends only ids, an intent, and how a
  source was reached.
  - **Searches:** `search_documents(query, intent)` hands every page to
    `Recorder::search`. Only `intent: submit` records; the default is
    `refresh`. The same words and filters as the last step update it.
  - **Questions:** `ask` takes an `AskRecord` when the answer starts
    (anchors, filters, model, where its service is). The answer thread
    returns an `AnswerOutcome` (the delivered text, how it ended, cited,
    dropped) and records after its last event. Nothing found is recorded
    in `ask`.
  - **Opens:** `record_open(fileId, via, action, place?)`. The anchor is
    built from the catalog, at the passage the user saw (the last page's
    and the last answer's passages are kept in memory). From the Library
    it records nothing. Consecutive opens of a document the same way merge.
  - **A failure** never fails the search or the answer: it sets
    `RecordingStatus.failed` until a step is saved again.
- **Steps** (`trails.rs`): `append` is one transaction. It applies the
  session rule, merges or inserts the step with its `step_documents`,
  recounts the trail, refreshes its folded `search_text`, and keeps the
  session going. Payloads are `StepPayload` JSON at `v: 1`.
- **Sessions:** `meta.current_trail` and `meta.current_trail_until` (the
  last step plus 30 minutes). `start_trail` clears them; `continue_trail`
  sets them.
- **Settings** (`config/settings.json`): `privacy.recordTrails` and
  `privacy.expireTrails`, both on by default, are shared with the recorder
  and the store's hourly thread through `TrailSettings`, updated by
  `update_settings`.
- **Expiry:** at open (after the snapshot check) and hourly, at most daily
  (`meta.last_expiry_at`). It removes unpinned trails in no collection
  whose last step is over 90 days old.
- **`trail-changed`** carries a `RecordingStatus` (recording, paused or
  unavailable; the current trail and its step count; Home's one-time note;
  `failed`) after each step and each change. `Workspace::with_trail_events`
  wires it, as the job events are wired.
- **The renderer:**
  - `search.svelte.ts` sends `submit` only from `submit()`.
  - `details.open`, `details.show` and the Open and Show actions take a
    `via` and call `record_open` after success (`openVia.ts`). The Library
    and the duplicates list pass none.
  - `trails.svelte.ts` holds the status and does the toggle, New trail,
    dismissing Home's note, and Delete all trails.
  - The status bar shows "Trail: recording" or "Trail: paused" as a toggle
    button, and "Atlas couldn't save this step" after a failure. Search has
    **New trail** beside the mode switch. Settings › Privacy has a Research
    trails part (`TrailPrivacy.svelte`).
- **Reading trails:** `list_trails` (paged, newest activity first,
  filtered on `search_text`) and `get_trail` (steps oldest first, every
  anchor resolved now).

### The Trails screen and resuming (Slice 3.4)

- **The view** (`views/Trails.svelte`, `stores/trailView.svelte.ts`) is the
  fourth main view (Cmd/Ctrl+4).
  - **The list:** a filter (debounced like the Library's), 25 trails a
    page, each with its label, dates, steps and a Pin toggle.
  - **An open trail:** `TrailTimeline.svelte`, oldest step first.
- **Steps as recorded:**
  - A search with its filters, the count then, and the top results.
  - A question with its answer through `AnswerText.svelte`: the paragraph
    and chip rendering taken out of `AnswerView`, not a live region. Then
    the ending, the weak and dropped notes, and Sources then.
  - A source opened, with how it was reached; a note.
  - Steps carry their own note, Add a note and Delete this step. The trail
    has Continue, Rename, Pin and Delete, and a note of its own at the end.
- **Sources** (`TrailSource.svelte`, `trailOpen.ts`):
  - A source that is still there opens in the details panel with
    `via: trail`, so the open is recorded. A question's source shows its
    excerpt as sent, with "Sent to the model for this question on …"
    (`details.open`'s `citedNote`).
  - `found_elsewhere` and an excerpt not found say so. `no_file` and
    `unknown` show the saved excerpt and their sentence, and don't open.
- **Search again and Ask again** call `continue_trail`, then put the
  recorded words or question and filters on the Search screen and submit
  (`search.apply`) or ask. A recorded folder that is no longer in the
  library is dropped, and `search.notice` says so once.
- **An open timeline** reloads when `trail-changed` reports a new step
  count for it.
- **Home** lists the three most recent trails under "Pick up where you
  left off"; each opens its timeline.

## Bookmarks and findings

Implemented in `src-tauri/src/services/knowledge/saved.rs` and
`src-tauri/src/commands/saved.rs` (Slice 3.5, phase3-spec §5.3, §7.3).

- **The renderer names what to keep; Rust builds it.**
  - `SaveTarget::File { fileId, place }`, where `place` is `result`,
    `citation{answerId, n}`, `page{pageNo}` or `document`. The anchor is
    built from the catalog at the passage the recorder remembers for that
    result or citation; a page's first passage; or the whole document. A
    citation of the answer on screen keeps its excerpt as sent.
  - `SaveTarget::Step { stepId, index }` keeps a trail step's anchor as it
    was recorded.
- **One bookmark per place:** the same `document_id` and anchor ordinal
  (`json_extract(anchor, '$.ordinal') IS ?`, so the whole document counts
  as a place) updates the bookmark already there.
- **Findings** (`FindingSource`):
  - `answer{answerId}` reads the recorder's last ended answer. It is kept
    in memory, recording or not, until the next question starts: the
    question, the delivered text, and the cited sources only, with the step
    id when it was recorded.
  - `step{stepId}` reads a recorded question.
  - `passage{target}` keeps one passage's text with its anchor.
  - Anything gone answers `save_source_gone`.
- **Notes** use the trails' 4,000-character check. **Deleting** also
  removes `collection_items`, in one transaction. Saving is never a trail
  step; opening a saved item is (`via: bookmark`).
- **`list_saved`** reads every bookmark and finding, filters by kind,
  document and folded text in Rust (no filter column; the counts are
  small), sorts by `updated_at`, then resolves the page's anchors.
  `SavedItem` is tagged `item`, because a finding has a `kind` of its own.
- **The renderer:**
  - `SaveAction.svelte` is a Bookmark or Save as a finding button with its
    status. It appears on search results, the details panel (the whole
    document; the cited passage as a bookmark and as a finding, from
    `details.open`'s `saveAs`), Sources used, a done answer, a trail
    question, and each recorded source.
  - The Trails view has Trails | Saved tabs. `SavedList.svelte` shows
    bookmarks and findings through `RecordedSource.svelte` (renamed from
    `TrailSource`) and `AnswerText`.
  - The details panel lists **Bookmarks in this document**.

## Interests, projects and rediscovery

Implemented in `src-tauri/src/services/knowledge/{collections,rediscovery}.rs`,
`src-tauri/src/services/documents/rediscover.rs` and
`src-tauri/src/commands/collections.rs` (Slice 3.6, phase3-spec §5.4,
§7.4).

- **Collections:**
  - An interest has a name (1 to 200 characters) and an optional
    description. A project adds an optional goal and can be finished.
    Either can be archived. Its kind never changes, and a goal or Finished
    on an interest is `invalid_collection`.
  - Items are links in `collection_items`: a trail, bookmark or finding by
    id; a document by its id and content hash (added by a file id).
    Adding twice is a no-op. Removing a link, or deleting the collection
    (its links cascade, and its dismissals are deleted), never deletes the
    item. Deleting a trail, bookmark or finding removes its links (Slices
    3.3, 3.5).
  - `get_collection` returns the trails' summaries, the bookmarks and
    findings as Saved shows them (`saved::by_ids`), and each document
    resolved now, with its catalog title.
- **Suggestions** (`get_suggestions`, at most six), computed each time and
  never stored:
  - **Seeds:** each active collection (its documents, its bookmarks' and
    findings' sources, its trails' `step_documents`), and the three most
    recent trails' documents (seed `''`).
  - **A seed's vector** is the re-normalised mean of its documents' Related
    vectors (`related::mean_vector`). The nearest 200 chunks of other
    documents (the seed's own filtered inside the index), at or above the
    model's similarity floor, are grouped by document, best first.
  - **Left out:** the seed's documents; documents opened in a trail (open
    steps' `step_documents`); documents dismissed for that seed; documents
    with no available file (`search::meaning_hit`).
  - **Order:** seeds sorted by a BLAKE3 hash of the clock's UTC date and the
    seed id, then round-robin, each document once: the same all day.
  - **No model is called.** Only stored vectors are read. With no
    embedding role, no serving model, or an index not open yet (it is
    opened in the background), there are none.
  - `dismiss_suggestion(seedId, documentId)` records Not for this, with the
    document's hash.
- **The renderer:**
  - The Trails view's third tab is **Interests and projects**
    (`CollectionList.svelte`; a collection's page is
    `CollectionDetail.svelte`).
  - `AddTo.svelte` (Add to…) is on a trail's timeline, each Saved item and
    the details panel. It lists the active collections with `aria-pressed`
    membership (`get_collection_membership`), and offers New interest… and
    New project….
  - Home shows the active collections (at most six, then All interests
    and projects) and up to three suggestions with their reason, Details
    (recorded `via: suggestion`) and Not for this. Each section shows only
    when it has something to show.

## Page view

Implemented in `src-tauri/src/services/reader/{mod,cache}.rs`,
`src-tauri/src/services/extraction/engine/render.rs`,
`src-tauri/src/commands/reader.rs`, `src/lib/stores/pageView.svelte.ts` and
`src/lib/components/PageView.svelte` (Slice 3.7, phase3-spec §5.5, §7.5;
[ADR 0018](adr/0018-the-page-view.md)).

- **A second worker.** `ReaderService` (one per open library, in
  `OpenRoot`) owns its own `Supervisor` over the app executable. It is
  started at the first render, killed after 60 s idle by its own thread,
  and dropped with the library. One render at a time. The extraction job
  never uses it, and it never uses the extraction worker.
- **In the worker** (`engine/render.rs`): Open's file checks (`hash::open`,
  the signature sniff), then PDFium renders the page at the width asked for
  (at most 2,000 px; the long edge scaled to at most 3,000), encodes RGBA
  to PNG (`png`, fast compression; at most 8 MiB), and finds the passage
  (whitespace collapsed, 200 characters, then 60), turning each match's
  segments into pixel rectangles. Not found is `found: false`, not an
  error. Debug builds' test faults (`hang`, `exit`, `bloat`) apply to
  renders too.
- **Limits:** the supervisor's 30 s inactivity and memory cap kill the
  worker, and the next render starts another. `WorkerStats` gains `renders`
  and `peak_rss_bytes`, which is also sampled after each render.
- **`render_page(fileId, pageNo, widthPx, find?, allowDownload?)`:**
  1. Under the workspace lock, `reader::prepare` looks up the document, its
     content hash and type (`render_failed` `not_read` or `not_a_pdf`), the
     page's text from its passages, and the path or Open's error through
     `source_open::path_for`.
  2. Off the lock, on a blocking thread: the width and page are checked
     (`invalid_render`); the cache is read; only then is the path's error
     returned (so a cached page shows for a file now only in the cloud);
     the worker renders; the page is cached.
  3. `PageImage { pageNo, pageCount, width, height, png (base64), rects,
     found, text, fromCache }`.
- **The cache:** `cache/previews/<content hash>/<page>-<width>-<find
  key>.png` with a `.json` (page count, size, rectangles), written beside
  and renamed. The find key is 8 hex characters of the BLAKE3 hash of the
  normalised text, or `none`. At most 200 MB (`ReaderConfig`): after each
  write the oldest by modification time go first, and a hit refreshes the
  time. Clean caches empties it; Storage counts it with the caches.
- **The renderer:**
  - **Show the page** is offered only for a PDF with a page number: on the
    details panel's cited passage (`find` = the excerpt), a PDF search
    result (the snippet's text) and a recorded source with a PDF anchor
    (bookmarks, trail steps, collections).
  - `PageView.svelte` is a modal dialog, mounted once in `App.svelte`. It
    has Previous and Next, a page field ("Page 12 of 340"), Fit width or
    Actual size, Bookmark this page (`file{fileId, page{pageNo}}`), Open in
    the default app and Close. Escape closes it, focus is trapped while it
    is open, and focus returns to the button that opened it.
  - The image is a `data:` URL with alternative text ("Page 12 of 340").
    The outline is an SVG over it (Slice 3.9): its rectangles are in the
    image's pixels, with a `viewBox` of the image's size, so they follow
    either size without inline styles. The page's text is shown below it. The
    passage is looked for only on the page it was cited on.
  - Showing a page records `record_open(…, 'page', {pageNo})` with the
    caller's `via`. A file only in the cloud asks Download and show, as
    Open does.
- **No PDFium in the main process:** P3-20 checks the loaded modules after
  rendering (Linux `/proc/self/maps`, macOS dyld). The check is skipped on
  Windows.

## Export and import

Implemented in `src-tauri/src/services/export/{mod,dest,markdown,json,import}.rs`,
`src-tauri/src/commands/export.rs`, `src/lib/stores/exportDialog.svelte.ts`,
`src/lib/components/{ExportDialog,ExportPanel}.svelte` (Slice 3.8,
phase3-spec §5.8, §7.7; [ADR 0019](adr/0019-export-and-import.md)).

- **Commands:** `export_knowledge(scope, format, linkStyle)`,
  `preview_import()` and `apply_import(token)`. Rust shows the folder and
  file dialogs, outside the workspace lock. `null` means the dialog was
  cancelled. `export::Handles` copies the store, the catalog reader, the
  data root and the library folders out under the lock, and the work runs
  without it.
- **Gathering** (`export::gather`) is one read of the knowledge store into
  the JSON format's types: a trail, a finding, a collection (with its
  trails, bookmarks, findings and "Not for this"), or everything. For
  Markdown, `export::places` resolves each document in the catalog for its
  title and its file's path as text.
- **The destination** (`dest.rs`, `ExportDest`):
  - **Refused:** a library folder or inside one, inside the data root but
    not its `exports`, and an `Atlas` (or subfolder) that is a file or a
    link.
  - **`claim`** takes a name: free, Atlas's own note with the same
    `atlas_id` (replaced), or else numbered beside what holds it.
  - **`write`** stages each file beside its name, then renames it into
    place.
  - **`finish`** flushes each folder once.
- **Markdown** (`markdown.rs`): every name is taken before anything is
  written, so links point at real names. Front matter is `atlas_id`,
  `atlas_kind`, `created`, `updated` and `exported`. Links are Obsidian's
  `[[Folder/Name]]` or relative Markdown links.
- **JSON** (`json.rs`): `KnowledgeFile { format, version 1, exported, scope,
  trails[steps], bookmarks, findings, collections[items],
  dismissedSuggestions }`, with `deny_unknown_fields` on every row and no
  paths.
- **Import** (`import.rs`):
  1. Read the file: at most 1 GiB (Slice 3.9; peak memory about 2.4 times
     the file), then parsed and checked.
  2. `merge(dry: true)` is the dry run.
  3. `Pending` holds one parsed file under a token for 10 minutes.
  4. `apply` takes a snapshot, then runs `merge(dry: false)` in one
     transaction, then emits `trail-changed`.

  Merging adds what is new and keeps the newer copy by `updated_at`. Trail
  steps are combined, then renumbered by time. Links and "Not for this" are
  only added.
- **Settings:** `config/settings.json` gains `export.lastFolder`. It is kept
  outside the `Settings` DTO, so the renderer never sees or sends it, and
  `settings::write` keeps it when the renderer saves.
- **The renderer:**
  - **Export…** on a trail's timeline, a finding in Saved, and a
    collection's page.
  - **Settings › Export and import:** Export… (everything) and Import from
    a file….
  - **The dialog** (`ExportDialog.svelte`, mounted once in `App.svelte`)
    asks what, the format, and for Markdown the link style. Choose a folder
    and export… runs it, and the dialog says where it wrote. Escape closes
    it and focus returns.
  - **Import** shows the dry run's sentence, then Import or Cancel.

## Diagnostics

Implemented in `src-tauri/src/services/diagnostics/` (Slice 4.1);
rationale in [ADR 0021](adr/0021-diagnostics.md).

- **Events** (`event.rs`): a closed enum of codes and numbers, with no
  field that can hold text made at run time. A panic's file is kept only
  as Atlas's own `src/…`, never a build machine's path.
- **The handle** (`Diagnostics`): one per workspace, shared in the app
  with the resource governor (policy changes) and the panic hook. It is
  attached when a library's lock is held, so migrations at open are
  logged, and detached by a guard that is the open library's last field.
  With no library open, nothing is written.
- **The writer** (`log.rs`): a thread behind a bounded queue (1,024).
  Drops are counted and written as one line. JSON lines go in
  `logs/atlas-<UTC date>.jsonl`, rolling at 5 MiB, the folder kept under
  50 MiB. A failed write stops the log until the library opens again.
- **What records:**
  - the job runner (start and end, with items and errors, and a failed
    job's reason as a closed code, Slice 4.1.1);
  - user and policy pauses, once per change;
  - the supervisors, tagged `read` or `render` (start, crash, refusal);
  - the governor (policy changes);
  - snapshots;
  - `Serialize for AtlasError` (every error returned to the window, by
    code);
  - the search command, the answer thread, and the vector registry (index
    loaded and let go).
- **The report** (`report.rs`): versions, the device check, settings
  modes, storage, counts (files, documents, pages needing OCR, passages,
  chunks, vectors per model by position, jobs), the knowledge store's
  counts or only its state, and the log's last 2,000 events. It is written
  to `exports/diagnostics/`, off the workspace lock, and works with the
  knowledge store damaged.
- **The renderer:** Settings › Diagnostics (`DiagnosticsPanel.svelte`,
  `diagnosticsCopy.ts`) says what a report holds and never holds, then
  offers Save a diagnostics report, Show in…, Show the log folder and
  Clear the log, which asks first (Slice 4.1.1). It sends only which
  folder to show.
- **The worker's hello** (Slice 4.1): a worker of another protocol is
  refused as `Unavailable(ProtocolMismatch)`. Reading ends with
  `worker_protocol`, which `RootExtractionSummary.failedBecause` carries
  to the status line.

## Build, CI and measurement

- **PDFium** (Slice 1.2) is pinned in `tools.lock.json` (bblanchon/pdfium-binaries
  chromium/8125, SHA-256 per platform). `scripts/fetch-pdfium.mjs` verifies
  and unpacks it into gitignored `src-tauri/vendor/pdfium/<platform>/` and
  collects its notices into `notices/`. tauri-build checks bundled files on
  every compile, so `just setup` and the CI setup action (cached by the lock
  file's hash) fetch it before any Rust build. The per-OS configs bundle it:
  `tauri.macos.conf.json` lists it under `bundle.macOS.frameworks`
  (`Contents/Frameworks`, signed with the app, with the
  `disable-library-validation` entitlement, see [packaging](packaging.md));
  `tauri.windows.conf.json` and `tauri.linux.conf.json` ship it as a
  resource. Every bundle carries the notices in `licenses/pdfium/`.
- **CI** (`.github/workflows/ci.yml`, shared setup in
  `.github/actions/setup`) has three tiers (Slice 1.1) and a manual
  evidence run (Slice 1.8). A `plan` job picks one and writes it to the
  run summary:

  | Tier | When | What runs | Wall time |
  | --- | --- | --- | --- |
  | full | Pushes to `main`, tags, pull requests, manual runs (`just ci-remote`) | Every job below on macOS arm64, Windows x64 and Linux x64; benchmarks only on pushes | About 12 min warm before merging (the Windows test job); 16–21 min on `main` with benchmarks |
  | quick | Pushes to `slice-*` branches, `just ci-remote quick` | Linux only: Rust format, clippy and tests, bindings, lint, typecheck, frontend tests | 2.5 min warm; 5 min with a cold cache (3.5 min of it compiling) |
  | docs | Branch pushes that change only `*.md`, `docs/` or `spikes/` | ESLint and Prettier (which checks Markdown) | 35 s |
  | evidence | `just ci-remote evidence` only | `evidence` (Phase 3, Slice 3.9): the knowledge store at 100,000 steps loaded through Import, with import's memory, and the page view, on all three OSes (`bench.mjs --evidence`). Phases 1 and 2's evidence (the 30k read, the 10k embedding) is `bench.mjs --evidence-phase2`. Its own concurrency group | 8 to 15 min |

  The full tier runs:
  - `check-and-build`: every check and test, then debug bundles. On Windows
    and Linux it also runs the WebDriver smoke test. On every OS
    `scripts/worker-hello.mjs` starts the extraction worker from the build
    and requires `hello`; on macOS it first runs
    `codesign --verify --deep --strict` on the `.app`.
  - `bench`: the benchmark suite, whose tables become the job summary.
    Numbers never fail the build. It runs only on pushes (`main` and tags),
    not on a slice's run before merging: at 11–20 min it was the slowest
    job (Slice 3.2).
  - `package`: unsigned release bundles, except on pull requests. The worker
    must say hello from the signed release `.app`, the unpacked `.deb`, the
    AppImage, and a silent per-user NSIS install.

  The quick tier shares the full Linux job's Rust cache (`rust-shared-key`
  in the setup action), so a new slice branch starts warm from `main`'s.
  Slice work runs quick on every push and full before merging, and `main`
  and tags always run full. Every job uploads its artifacts. Timeouts are
  kept close to the real times, so a stalled runner fails within about
  half an hour: checks and tests 30 min, benchmarks 30, release bundles
  20, quick 45. The setup action bounds the Linux package install to 5
  minutes, then retries from `archive.ubuntu.com` for up to 10 (the runner
  mirror failed for hours in Slice 3.2). See
  [packaging](packaging.md).
- **Smoke test** (`e2e/smoke.mjs`). A dependency-free W3C WebDriver client
  drives the debug build through `tauri-driver`. The flow covers
  onboarding, adding a folder, pause and resume, completion, name search,
  open, quitting while text is being read (no `--extract-worker` process may
  outlive the app), pausing and resuming the reading after the relaunch,
  searching a passage from Home on the 1,000-file text fixture
  (`--text-kb 4`) and opening that result, duplicates, settings, and a
  relaunch. Phase 3 (Slice 3.8) adds: the search and question recorded in a
  trail; after the relaunch, the trail resumed and its citation opened; a
  bookmark saved; the page view showing a PDF page; and the trail exported
  to a scratch folder, its note checked on disk. It waits for the native
  driver's port and retries a session the driver refuses once, recording
  the retry (Slice 1.8: msedgedriver sometimes refused the first
  connection on Windows runners). WebDriver can't answer native folder dialogs
  or observe the OS opening a file, so debug builds read
  `ATLAS_E2E_PICK_DATA_ROOT`, `ATLAS_E2E_PICK_SOURCES`,
  `ATLAS_E2E_PICK_EXPORT_DIR`, `ATLAS_E2E_PICK_IMPORT_FILE` (Slice 3.8) and
  `ATLAS_E2E_OPENER_LOG` (`infrastructure/e2e.rs`), and catalogs at a fixed
  150 entries a second (`ATLAS_INVENTORY_RATE`), so the pause step always
  finds the scan running. Release builds ignore them, as they ignore
  `ATLAS_CONFIG_DIR`. On Windows, wry always sets
  WebView2's browser arguments, which hides the remote-debugging arguments
  msedgedriver passes through `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`. So
  the main window is created in `setup` (`"create": false` in
  `tauri.conf.json`), and debug builds append the driver's arguments to
  wry's defaults. macOS has no WebDriver for WKWebView, so it uses the
  manual checklist.
- **Benchmarks** (`src-tauri/benches/`, driven by `scripts/bench.mjs`).
  Release build, real filesystem and SQLite, and simulated power for
  repeatable AC and battery runs. `inventory.rs` catalogs (following the
  runner's progress events instead of polling, so it measures cataloging
  rather than itself), `search.rs` searches while text is read, and
  `extraction.rs` (Slice 1.7) reads a text fixture at the Run and Slow duty
  cycles and unthrottled, sampling the app and the worker, and kills a
  child mid-read to check recovery. CI runs all three on the 1k fixtures;
  the 30k text fixture runs locally (`just bench-search`,
  `just bench-extraction`). `reader.rs` (Slice 3.7) renders the corpus's
  PDF pages in the reader's worker (`just bench-reader`, and in CI's bench
  job). `knowledge.rs` (Slice 3.8) loads a store from
  `scripts/generate-knowledge.mjs` through Import and measures the Trails
  list, a 50-step trail, recording, a snapshot and export
  (`just bench-knowledge`; CI runs 10,000 steps). Method and results:
  [benchmarks](benchmarks.md).

## The Explore spike (Slice 4.1.2)

`src-tauri/src/services/explore/` is a spike, not a feature: pure
functions that lay a library out by meaning, called by nothing in the
app. A harness (`src-tauri/tests/explore.rs`, `just spike-explore`) runs
them against a library and writes one page for its owner to judge. The
question, the rule and the results are in
[spike-explore.md](spike-explore.md); if the answer is stop, the module is
removed.

- **`books.rs`:** one unit vector per book for one embedding model, the
  mean of the sample Related documents averages, read in one short
  statement per document.
- **`cluster.rs`, `knn.rs`, `layout.rs`:** areas by bisecting spherical
  k-means, exact nearest neighbours on threads, and a 2-D layout (PCA,
  then UMAP's pulls and pushes). Seeded; no dependency.
- **`quality.rs`, `names.rs`:** measures of the areas and the map, and
  area names from the books' own words.
- **`survey.rs`:** the pipeline, and `Numbers`, which holds counts,
  measures and timings only.
- **`catalog::reader::open_offline`:** a catalog opened from outside
  Atlas, read-only, immutable when Atlas is closed, never migrated, and
  refused at any schema but the build's.

## Spikes (Slice 0.9)

[`spikes/`](../spikes/README.md) holds experiments that inform decisions
and are not part of the app: `src-tauri` depends on nothing there, and
`ci.yml` doesn't build it. The phase0 spec's layout names
`spikes/extraction-bench/`; the vector comparison sits beside it in
`spikes/vector-bench/`. Both are separate Cargo workspaces with their own
lockfiles, so each candidate's build time and binary size can be measured
alone.

- **Extraction** candidates are small binaries (or, for Docling, a frozen
  Python program) that share one contract: file paths in, one JSON line per
  file out. A driver runs every document in its own process (to record
  crashes and hangs) and then all of them in one process (throughput), and
  a scorer compares the text with the corpus truth.
- **Vector** engines implement one `Engine` trait (build, reopen, search,
  filtered search, delete, crash-insert, crash-verify) and run the same
  scenario on the same synthetic data with exact ground truth.
- **CI**: `.github/workflows/spikes.yml` runs the drivers on all three OSes
  when dispatched by hand. It uses the shared setup action with
  `app: 'false'` (no WebKitGTK or npm packages) and `rust-cache: 'false'`
  (cold builds are part of what's measured).
- **Tools** (PDFium, protoc, typst) are pinned in `spikes/tools.lock.json`
  and downloaded with their SHA-256 checked into `spikes/.tools/`; nothing
  is installed system-wide.

The decision is [ADR 0008](adr/0008-extraction-and-vector-engines.md);
the measurements are in [the spike results](spike-0.9.md).

## Reproducibility

- `rust-toolchain.toml` pins Rust 1.95.0. `Cargo.lock` is committed and CI
  passes `--locked`.
- `package.json` pins exact npm versions and the pnpm version.
  `pnpm-lock.yaml` is committed and CI uses `--frozen-lockfile`.
- GitHub Actions are pinned to full commit SHAs.
- Native libraries the app ships (PDFium) are pinned with SHA-256 in
  `tools.lock.json`; the spike's tools in `spikes/tools.lock.json`.
- Fixtures are deterministic: the same generator arguments give the same
  bytes on every machine (`scripts/generate-fixtures.test.mjs`).

## Next phase

Phase 1 (complete) added a general jobs table, text extraction in a helper process
(PDFium and rbook), content hashing with logical documents, and FTS5
full-text search. It is specified in [phase1-spec.md](phase1-spec.md).
Slice 1.1 built the jobs table and runners ([Jobs](#jobs)); Slice 1.2 the
text fixtures, PDFium delivery and the [extraction worker](#extraction-worker);
Slice 1.3 [extraction](#extraction) itself; Slice 1.4 [search](#search);
Slice 1.5 the Search screen and document details ([Frontend](#frontend));
Slice 1.6 [metadata, duplicates and storage](#metadata-duplicates-and-storage);
Slice 1.7 the carry-overs: cheaper replays when resuming inside a large
folder ([Inventory](#inventory)), the worker's [lifetime](#extraction-worker),
16 KiB catalog pages ([Data root](#data-root)), and the extraction
benchmark; Slice 1.8 the evidence and the
[close-out review](phase1-closeout.md) (version 0.2.0).

Phase 2 (the AI librarian: embeddings through a connected model service,
a usearch index derived from the catalog, hybrid search, related
documents, and Ask Atlas with verified citations) is specified in
[phase2-spec.md](phase2-spec.md). Slice 2.1 built the model roles and
capability tests ([AI service connections](#ai-service-connections));
Slice 2.2 the chunks, durable embeddings and the vector index
([Embeddings and the vector index](#embeddings-and-the-vector-index));
Slice 2.3 the embedding job that fills them; Slice 2.4 hybrid
[search](#search); Slice 2.5 [related documents](#related-documents);
Slice 2.6 the [Ask Atlas](#ask-atlas) engine and the privacy choices;
Slice 2.7 the Ask Atlas screen; Slice 2.8 the evidence and the
[close-out review](phase2-closeout.md) (version 0.3.0).

Phase 3 (the knowledge workspace: research trails, bookmarks and saved
findings, interests and projects, a PDF page view, export) is specified in
[phase3-spec.md](phase3-spec.md). Slice 3.1 built the carry-overs
(iCloud re-reads, the per-model weak margin, Project Gutenberg boilerplate
left out), 3.1.1 the fusion change, 3.2 the
[knowledge store](#knowledge-store), 3.3
[recording trails](#recording-trails), 3.4 the Trails screen and resuming,
3.5 [bookmarks and findings](#bookmarks-and-findings), 3.6
[interests, projects and rediscovery](#interests-projects-and-rediscovery),
3.7 the [page view](#page-view), 3.8 [export and import](#export-and-import),
and 3.9 the evidence and the [close-out review](phase3-closeout.md)
(version 0.4.0).

Phase 4 (full-collection readiness: OCR of scanned PDFs, automatic
rescans, schedules for background work, checks and repairs, backups and
restore, Move Atlas data, a local log and a diagnostics report, and the
product owner's run against the real collection) is specified in
[phase4-spec.md](phase4-spec.md). Slice 4.1 built
[diagnostics](#diagnostics) and the carry-overs: the worker's protocol
checked, page counts kept for PDFs that need OCR (migration 0015), and the
meaning index's lock split (one cause of the P3-18 flake; another is
still unexplained).
