# Atlas architecture

Status: Phase 1, Slice 1.1 (general jobs). Phase 0 is complete; the rest of
Phase 1 is specified in [phase1-spec.md](phase1-spec.md) and not yet built.
This document describes what is built, and marks what is only planned. Decisions with lasting
consequences are recorded as ADRs in [adr/](adr/).

## Process model

Atlas is one desktop process. The Tauri 2 runtime hosts the native window, an
OS webview that renders the Svelte UI, and the Rust core. There is no HTTP
server, background daemon, database server, or Python runtime.

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
| `ModelProvider` | Endpoint probe and model listing | **0.7 (`HttpModelProvider`)** |
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
- Source files are never opened through string-built shell commands.

## Frontend

Svelte 5 with runes and Vite, without SvelteKit, because there is no SSR or
routing need. A small store selects the current view. At launch,
`src/lib/boot.ts` asks the core for the data-root status and routes:
not configured goes to Welcome and then the data-location screen, open goes
to Home, and missing, in use, incompatible or invalid goes to a recovery
screen. The theme is saved in the data root's `config/settings.json`;
browser storage is not used for Atlas preferences.

Accessibility baseline: landmarks and a skip link, `aria-current` on
navigation, focus moved to each view's heading, Cmd/Ctrl+1–3 view shortcuts,
visible focus rings, 44px major controls, `prefers-reduced-motion`, and
system light/dark via `prefers-color-scheme` with a `data-theme` override.

## Data root

Implemented in `src-tauri/src/services/workspace/`. Design rationale is in
[ADR 0002](adr/0002-data-root-creation-locking-and-compatibility.md).

```text
<root>/atlas.json             manifest: root ID, layout and schema versions (written last)
<root>/atlas.lock             exclusive OS lock while a process has the root open
<root>/config/settings.json   portable preferences (theme)
<root>/library/catalog.sqlite SQLite catalog, WAL mode (+ -wal, -shm)
<root>/tmp/                   staging for atomic writes; emptied on open
<root>/content/ indexes/ assets/ cache/ jobs/ models/ exports/ logs/ backups/   reserved, empty
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
  busy timeout. Migrations in `src-tauri/migrations/` run in order, each in
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
  bucket. Classification is by extension only, and file contents are never
  opened.
- **Triggers.** A session starts when a folder is added, on Rescan, or when
  a library opens with interrupted work or a never-scanned folder. User
  pauses persist across restarts.
- **Missing files.** A file is marked missing only when its folder was
  listed successfully in a session and the file wasn't found. A folder
  that can't be read protects its whole subtree.
- **Cloud files.** iCloud dataless files, legacy `.icloud` placeholders,
  and Windows cloud placeholders are cataloged as `cloud_only` without
  being downloaded. Listing a dataless folder has a 15 second timeout.
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
[ADR 0007](adr/0007-provider-connections-and-secrets.md).

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
- **Tests** are bodiless GETs of the model list, with no redirects, no
  proxies, bounded time and size, and honest outcomes. They never claim
  chat or embedding support.
- **Detection** probes loopback well-known ports only, and only when the
  user asks.

The webview also creates its own website-data and cache folders, on macOS
`~/Library/WebKit/<bundle>` and `~/Library/Caches/<bundle>`. These are
platform-required and contain no Atlas data.

## Build, CI and measurement

- **CI** (`.github/workflows/ci.yml`, shared setup in
  `.github/actions/setup`) has three tiers (Slice 1.1). A `plan` job picks
  one and writes it to the run summary:

  | Tier | When | What runs | Wall time |
  | --- | --- | --- | --- |
  | full | Pushes to `main`, tags, pull requests, manual runs (`just ci-remote`) | Every job below on macOS arm64, Windows x64 and Linux x64 | 7–9 min warm; 10 min with cold caches |
  | quick | Pushes to `slice-*` branches, `just ci-remote quick` | Linux only: Rust format, clippy and tests, bindings, lint, typecheck, frontend tests | 5 min with a cold cache (3.5 min of it compiling); less once warm |
  | docs | Branch pushes that change only `*.md`, `docs/` or `spikes/` | ESLint and Prettier (which checks Markdown) | 35 s |

  The full tier runs:
  - `check-and-build`: every check and test, then debug bundles. On Windows
    and Linux it also runs the WebDriver smoke test.
  - `bench`: the benchmark suite, whose tables become the job summary.
    Numbers never fail the build.
  - `package`: unsigned release bundles, except on pull requests.

  The quick tier shares the full Linux job's Rust cache (`rust-shared-key`
  in the setup action), so a new slice branch starts warm from `main`'s.
  Slice work runs quick on every push and full before merging, and `main`
  and tags always run full. Every job uploads its artifacts. See
  [packaging](packaging.md).
- **Smoke test** (`e2e/smoke.mjs`). A dependency-free W3C WebDriver client
  drives the debug build through `tauri-driver`. The flow covers
  onboarding, adding a folder, pause and resume, completion, search, open,
  settings, and a relaunch. WebDriver can't answer native folder dialogs
  or observe the OS opening a file, so debug builds read
  `ATLAS_E2E_PICK_DATA_ROOT`, `ATLAS_E2E_PICK_SOURCES` and
  `ATLAS_E2E_OPENER_LOG` (`infrastructure/e2e.rs`). Release builds ignore
  them, as they ignore `ATLAS_CONFIG_DIR`. On Windows, wry always sets
  WebView2's browser arguments, which hides the remote-debugging arguments
  msedgedriver passes through `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`. So
  the main window is created in `setup` (`"create": false` in
  `tauri.conf.json`), and debug builds append the driver's arguments to
  wry's defaults. macOS has no WebDriver for WKWebView, so it uses the
  manual checklist.
- **Benchmarks** (`src-tauri/benches/inventory.rs`, driven by
  `scripts/bench.mjs`). Release build, real filesystem and SQLite, and
  simulated power for repeatable AC and battery runs. The harness follows
  the runner's progress events instead of polling, so it measures
  cataloging rather than itself. Method and results:
  [benchmarks](benchmarks.md).

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

## Next phase

Phase 1 adds a general jobs table, text extraction in a helper process
(PDFium and rbook), content hashing with logical documents, and FTS5
full-text search. It is specified in [phase1-spec.md](phase1-spec.md).
Slice 1.1 built the jobs table and runners ([Jobs](#jobs)). The rest is not
in the code yet; this document is updated slice by slice as it lands.
