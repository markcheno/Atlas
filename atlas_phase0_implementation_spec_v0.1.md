# Atlas — Phase 0 Implementation Specification v0.1

**Status:** Ready for coding-agent implementation  
**Date:** 2026-11-07  
**Parent:** [Atlas Product Specification v0.2](atlas_product_spec_v0.2.md)  
**Target platforms:** macOS arm64, Windows x64, Linux x64  
**Product:** Atlas Community — local-first personal knowledge discovery  
**Milestone:** A real, installable, useful desktop *foundation*, not a full AI research product

> **Product promise:** Collect freely. Organize minimally. Retrieve intelligently.  
> **Experience rule:** Simple by default; powerful by choice. No AI knowledge or terminal required for ordinary users.

## 1. Mission and definition of done

Build a polished, cross-platform desktop shell that allows a user to select existing folders of PDFs and EPUBs, choose a dedicated Atlas data location, inventory those files safely and incrementally, browse the catalog, and open the originals. The app must respect battery and storage constraints, survive interruption, and test a user-configured local AI endpoint. Provide a reproducible build/test/packaging foundation for later phases.

Phase 0 is complete **only** when the full vertical slice works end to end on each of the three target OSes:

1. A clean install or development launch presents first-run onboarding.
2. The user chooses an Atlas data root and at least one document folder; both persist across restarts.
3. An initial hardware/storage/power check yields intelligible, nonblocking guidance.
4. A throttled inventory job discovers PDF/EPUB file instances, persists results in SQLite **inside** the data root, and updates the Library view progressively.
5. A document can be opened by the OS default application from its recorded path.
6. The user can pause/resume; unexpected termination resumes without corrupting or duplicating records.
7. Battery/low-space/source-disconnect states are handled safely and explained plainly.
8. The user can enter a local model-service URL, test it, and view an honest connection result. AI usage is **not** necessary to inventory or browse.
9. Unit/integration/desktop smoke tests and packaging evidence are recorded for macOS, Windows, and Linux.

**Do not mark Phase 0 complete based on builds alone if installation/launch has not been smoke-tested on a platform.** If an OS is unavailable, document it as unverified, with a reproducible checklist.

## 2. Scope boundaries

### Implement in Phase 0

- Tauri v2 desktop shell, Svelte + TypeScript, a simple responsive UI, light/dark/system theme, accessible keyboard navigation.
- Onboarding: Welcome → Data location → Library folders → Device check → Initial inventory → Library.
- Multiple local source roots; add/remove roots from Settings; **remove from Atlas** never deletes source files.
- Persistent, read-only PDF/EPUB *inventory* with basic metadata: filename, extension/type, source path, file size, modified time, availability, discovery/update times. A logical-document abstraction may be defined, but is not populated yet.
- SQLite catalog, root manifest, versioned schema migration, durable directory frontier and scan state.
- Progressive Library list, filtering by source root and file type, sorting by name/date; filename/path matching. Basic search need not index document bodies.
- Safe “Open Original” and “Reveal in File Manager” actions using OS-native facilities.
- Storage and device preflight, battery-aware/resource-throttled scan scheduler, pause/resume/cancel, and simple progress and error reporting.
- Provider connection test for **Ollama** and **OpenAI-compatible** endpoints; URL and model listing where supported. Optional API keys stored in OS credentials, never plaintext. No model download, inference, or document upload.
- Modular interfaces for future parsers, AI providers, system probes, storage, and jobs (built-in implementations only).
- CI, automated tests, developer instructions, dependency/license inventory, basic build artifacts.

### Explicitly defer

Full PDF/EPUB text extraction; OCR; text search across document contents; deduplication by hash; thumbnails/covers; embeddings; semantic search; chat/RAG/citations; automatic tagging; Obsidian integration; interests/projects/trails; model downloads; embedded PDF reader; public HTTP API; user authentication; cloud sync; encryption beyond OS protections; background daemon after quitting; third-party plugin loading; app auto-updates; installer signing/notarization for public distribution.

**Scope discipline:** Provide stable seams for later work, but do not introduce a general framework or service because it *might* be useful. Prefer one executable/process and one SQLite database. Never run a separate database server.

## 3. Technology choices (Phase 0 defaults)

| Concern | Phase 0 choice | Notes |
| --- | --- | --- |
| Desktop application | Tauri 2 | Native webviews and OS packaging |
| UI | Svelte + TypeScript + Vite | Use maintained official scaffolding; no SSR requirement |
| Core/backend | Rust | Native filesystem operations, scheduler, SQLite, preflight, model probes |
| Local database | SQLite via maintained Rust binding (e.g. `rusqlite`) | Bundled SQLite if needed; migrations tracked in DB |
| Package manager | `pnpm` for UI; `cargo` for Rust | Commit lockfiles; no runtime installation needed for end users |
| Native folder dialog | Tauri Dialog | Do not expose arbitrary filesystem access to the renderer |
| Opening source files | Tauri Opener / OS-native opening | Only recorded documents, with permission/path validation |
| Device signals | Rust OS APIs / maintained telemetry crates | Unknown power/thermal state is permitted and shown |
| Provider checks | Rust HTTP client behind `ModelProvider` interface | Limited, cancellable request; TLS validation |
| Tests | Rust unit/integration, Vitest, frontend component tests, OS smoke tests | Add E2E smoke automation where reliable |

**Rule:** Verify current package compatibility, minimum Rust version, transitive licenses, and Tauri v2 permission configuration before pinning dependencies. Never use floating versions in the committed release configuration. All documentation should reflect what was actually implemented rather than imagined functionality.

## 4. Repository layout

Use a single repository with clear internal boundaries, roughly:

```text
atlas/
  README.md
  CONTRIBUTING.md
  docs/
    PHASE0.md                  # this specification
    architecture.md           # chosen decisions, design tradeoffs
    dependency-licenses.md    # dependencies, SPDX IDs, redistribution notes
    platform-validation.md    # macOS / Windows / Linux evidence and limitations
    adr/                      # short architectural decision records
  fixtures/
    README.md                 # generate local test fixtures; no copyrighted books
  scripts/
    generate-fixtures.*       # small synthetic PDF/EPUB/unsupported samples
  src/                        # Svelte frontend
    lib/
      api/                    # typed client for Tauri commands
      components/
      stores/
    routes-or-views/
  src-tauri/
    capabilities/
    src/
      main.rs
      lib.rs
      commands/              # thin UI IPC adapters
      domain/                # DTOs and interfaces
      services/
        workspace/           # data root lifecycle, manifest, locator
        catalog/             # SQLite persistence/migrations
        inventory/           # traversal and file metadata
        scheduler/           # durable work and policy evaluation
        system/              # power, CPU/RAM/disk readings
        providers/           # Ollama and OpenAI-compatible probes
        source_open/         # validated open/reveal actions
      infrastructure/        # concrete OS/database/HTTP adapters
    migrations/
    tauri.conf.json
  tests/
    integration/
  .github/workflows/
    ci.yml
```

The precise folder names may change for idiomatic Rust/Svelte organization, but preserve these responsibilities. **Do not create networked microservices or an HTTP server** in Phase 0. Tauri IPC is the private local API; keep service signatures transport-independent so a future versioned HTTP API can wrap them.

## 5. User experience and screen contracts

### 5.1 Welcome

- Headline: **“Explore everything you've collected.”**
- One primary action: **Get Started**.
- Explain in one sentence: “Atlas catalogs your existing files without moving or changing them.”
- No sign-in, feature-tour carousel, technical terminology, or AI configuration gate.

### 5.2 Choose Atlas data location

- Offer an OS-appropriate default and **Choose another location**.
- Display free disk space, read/write check, and a short explanation: “Atlas stores its catalog and generated data here. Your books stay where they are.”
- If an existing, compatible data root is chosen, reopen it instead of silently overwriting.
- If a root belongs to a newer unsupported schema, refuse writes and explain recovery.
- Detect when destination is inaccessible, not writable, or too low on disk space.
- Persistence is atomic; do not leave a half-initialized root that appears valid.

### 5.3 Add library folders

- Select one or more source folders using native dialog; show removable/absent source as unavailable without deleting its catalog.
- Make read-only policy explicit; no manual tagging or folder reorganization.
- Disallow selecting the Atlas data root itself as a source. If the data root is nested inside a source, exclude its entire subtree from discovery.
- Reject a newly selected source root nested inside an already selected root (or vice versa) with a clear explanation. This avoids accidental double-scanning; existing overlapping roots from migrated data must still be handled deterministically.

### 5.4 Device check

- Show RAM, CPU/core count, space on **Atlas data volume**, battery/charging when known, and a plain-language recommendation.
- Provisional guidance only: 8 GB RAM targets basic catalog use; local AI requirements depend on model. Do not make a hard hardware blockade at 8 GB.
- If battery state is unavailable, show **Unknown** and use a conservative, safe scheduler profile.
- Let user continue in catalog-only mode without AI or model endpoint.

### 5.5 Library

- Minimal navigation: **Home**, **Library**, **Settings**. Home may simply show a welcoming search field plus library progress; do not fake semantic search.
- Library rows: title/filename, type, source folder, size, modified date, availability; clear Open and Reveal actions.
- Search by filename/path in Phase 0; label this accurately, not “AI search.”
- Browse and filter while scanning; show partial results immediately.
- Empty state: “Choose a folder to start discovering your library.”

### 5.6 Indexing status

- Compact, persistent status: e.g. **“Cataloged 1,204 files · Scanning Books”** or **“Paused on battery”**.
- Controls: **Pause**, **Resume**, **Cancel**. Define Cancel as stopping the job, preserving already cataloged entries; never deleting sources.
- Show counts for discovered compatible files, completed folders, failures, and elapsed time; **do not invent a total or percentage before traversal finishes**.
- UI remains interactive when background scanning is active.

### 5.7 AI Models settings (connection spike)

- Choose **Ollama** or **OpenAI-compatible endpoint**; default Ollama URL is `http://localhost:11434`.
- URL, optional API key, and **Test Connection**. Show model IDs if returned, or an honest reason unavailable.
- Cloud/remote setup is optional; no silent network calls except when user explicitly tests an endpoint. No document text is sent.
- An internet endpoint must use HTTPS; loopback HTTP allowed. Plain HTTP to LAN address requires an explicit warning/approval. Mask API keys.
- **Never** claim chat or embedding compatibility merely because `/models` responds; test confirms reachability/model listing only.
- Future separate chat/embedding assignments are represented in the domain interfaces but not exposed as working features yet.

### Visual and accessibility rules

Use whitespace, restrained typography, one prominent primary action per screen, system fonts, macOS-like polish without violating Windows/Linux conventions, native focus behavior, keyboard tab order, readable status labels, minimum 44px touch-friendly major controls where practical, and system light/dark mode. No alerts for routine progress. Errors must include a recovery action.

## 6. Data root, lifecycle and persistence

One configurable **Atlas Data Root** contains all Atlas-generated persistent data (exceptions: OS credential storage, OS-native installation/runtime caches, and a tiny OS-managed locator path). Use this Phase 0 structure:

```text
<ATLAS_DATA_ROOT>/
  atlas.json                   # root UUID, app/layout/schema versions; no secrets
  config/settings.json        # portable nonsecret preferences
  library/catalog.sqlite      # catalog, source roots, durable scan state
  content/extracted/          # reserved for Phase 1+
  indexes/vectors/            # reserved for Phase 2+
  assets/thumbnails/          # reserved
  cache/previews/             # reserved
  jobs/checkpoints/           # reserved if file-based checkpoints needed
  models/                     # reserved for future Atlas-managed models
  exports/                    # reserved
  logs/                       # redacted diagnostics; disabled/minimal by default
  tmp/                        # atomic staging
  backups/                    # reserved; user-created backups come later
```

No other Atlas-owned persistent indexes or generated metadata may be scattered into watched source folders. A file containing only the data-root locator may live under OS application settings; absolute source paths are references and never automatically copied into the data root. Externally installed Ollama/LM Studio storage is not Atlas-managed.

**Required lifecycle behavior:**

- Bootstrap with an atomically written manifest (temporary file + flush/rename as supported), a unique root UUID, and a supported schema version.
- Keep relative internal paths relative to the data root. Store source file paths separately using platform-appropriate path representations; avoid lossy Unicode conversion where possible.
- Use SQLite transactions and migrations. Enable foreign keys, set a measured `busy_timeout`, and choose WAL mode with an explicit single-writer policy. WAL/SHM belong beside the DB.
- Prevent two Atlas processes writing the same root at once. Prefer an OS-released lock scoped to the root (not naive deletion of a stale lock file). Surface “Library already in use” clearly.
- If an external data drive disappears, **do not recreate an empty Atlas directory at the same path**. Stop and ask the user to reconnect/select the library.
- Do not delete original source files, modify file times, write sidecars, or create `.atlas` files in source folders.
- No automatic data-root relocation in Phase 0; the specification must preserve the future Move Atlas Data contract. If users choose a different root in Phase 0, treat it as opening/creating a distinct root, not a silent migration.

### 6.1 Minimum SQLite schema (adjust naming, preserve semantics)

```text
schema_migrations(version PK, applied_at)
workspace(id PK, name, created_at)                    # single local workspace initially
source_roots(id PK, workspace_id, path, status, created_at, last_scan_at)
file_instances(id PK, source_root_id, path, display_name,
               extension, file_type, size_bytes, modified_at,
               availability, last_seen_scan_id, created_at, updated_at)
scan_sessions(id PK, source_root_id, state, started_at, finished_at,
              scanned_directories, discovered_files, error_count, last_error)
scan_directories(id PK, session_id, path, state, attempts, last_error)
app_jobs(id PK, kind, state, priority, created_at, updated_at,
         retry_count, resume_data, last_error)
provider_profiles(id PK, kind, label, base_url, keychain_ref, created_at, updated_at)
```

Add uniqueness for file instance within a source root by canonicalized/native path identity **without lowercasing paths on case-sensitive volumes**. Stable UUIDs belong to records, not generated from absolute paths. Support resumable scan sessions and schema migration from the first release. Derived catalogs can be rebuilt; future user-authored notes/trails must be independently protected.

**Phase 0 metadata rules:** classify PDF/EPUB conservatively by extension and lightweight file signature where appropriate; do not parse book text, hash all file contents, or attempt logical work/edition deduplication. File moves may appear as missing + new during Phase 0; this limitation must be stated.

## 7. Inventory engine and durable scheduler

### 7.1 Scan algorithm

- One background traversal worker per active data root, initially **one source scan at a time**. Keep memory bounded; never collect every filesystem path into a giant in-memory list.
- Persist a directory **frontier** in `scan_directories`. To process a directory, stream its immediate entries in **bounded batches**, upsert supported file instances and enqueue child directories transactionally, then mark the directory completed only after traversal finishes. If interrupted, replay that directory safely using idempotent upserts; do not depend on serializing a native filesystem iterator. Never buffer an unbounded directory in RAM.
- Skip symlinked directories and file symlinks in Phase 0 to avoid cycles and escaping chosen roots; record/report skipped counts. Do not traverse into the Atlas data root even if it is below a source.
- New overlapping source roots are rejected at selection; if migrated/existing catalog data contains overlaps, do not allow runaway duplicate traversal, and document how source-root provenance is retained.
- File inventory consists of directory enumeration and lightweight metadata reads. No full file reads or content hashing.
- Use path + file size + modification time for an initial inexpensive change fingerprint. On rescans, upsert unchanged items without duplicate records; robust rename/dedup detection is a later phase.
- Only mark files **missing** after a scan of that source root completes successfully. A disconnected drive, cancelled scan, or permission error must **not** make the whole library appear deleted.
- After an unexpected exit, resume pending/running jobs in a recoverable queued state; process unfinished directories idempotently.
- Isolate unreadable files and directories, report errors, and continue safely; retries with a hard cap and backoff. Don't infinitely rescan a failing directory.

### 7.2 Power/resource policy

- Prioritize user interaction over background work. Inventory runs on a bounded worker, with rate-limited filesystem metadata operations and cooperative yielding; no unbounded thread pool.
- Starting profile to *benchmark and tune*: AC ≤100 file metadata operations/second; battery ≤20/second; one traversal worker; chunked transactions (e.g., up to 100 results). Rates are engineering starting values, **not measured guarantees**.
- On battery with charge **≤20%**: pause all bulk inventory. On battery above threshold: lightweight inventory may continue slowly; heavy OCR/embedding is not in Phase 0 and would pause by default later.
- On charging/AC: run conservatively; a user can always Pause. If power state is unknown: conservative rate, show Unknown; do not falsely assume AC.
- Pause new write batches if free space is below `max(2 GiB, min(10 GiB, 2% of volume capacity))`, or a required data-root volume is absent. Recheck after recovery; surface action clearly.
- When OS supports high memory pressure/thermal warnings, slow/pause; otherwise use bounded work and report unsupported telemetry, not fabricated values.
- Check shutdown/cancellation between batches; flush consistent checkpoints. Quitting the application stops background work by default; no hidden daemon.
- Do not request prevent-sleep privileges or override OS energy policies in Phase 0.

### 7.3 State machine

```text
queued -> running -> completed
            |  |  
            |  +-> paused_policy (battery/space/missing root) -> queued
            +----> paused_user -> queued
            +----> failed_retryable -> queued (bounded retries)
            +----> failed_terminal
            +----> cancelled
```

Represent reason codes separately from display text. Distinguish `paused_user` from `paused_policy`, so reconnecting a power cable never resumes a job that the user explicitly paused. Prior results remain browsable while scanning is paused.

## 8. Cross-platform system probes and minimum hardware

Implement a mockable `SystemProbe` interface returning a versioned, timestamped snapshot with optional fields:

```ts
type DeviceSnapshot = {
  os: 'macos' | 'windows' | 'linux';
  arch: string;
  logicalCores?: number;
  totalMemoryBytes?: number;
  availableMemoryBytes?: number;
  dataVolumeFreeBytes?: number;
  dataVolumeTotalBytes?: number;
  power: 'ac' | 'battery' | 'unknown';
  batteryPercent?: number;
  sampledAt: string;
};
```

- Prefer system APIs and maintained Rust crates with verified support per OS; use dependency injection/fake probes for tests. Do not equate “no detected battery” with “on battery.”
- Evaluate only the selected data-root volume for the storage guard; source drives may be different volumes.
- If a probe is unavailable, use a conservative fallback and allow catalog browsing; report missing telemetry in diagnostics.
- Min 8 GB RAM is an **unverified planning target**, not a hard check or guarantee. Benchmarks in Phase 0 establish truthful specifications for future releases.
- Recheck on startup, before starting/resuming a scan, on wake/power changes when supported, and periodically while running.

## 9. Service interfaces and extension-ready design

Create domain interfaces in Rust behind thin Tauri commands. No arbitrary runtime plugin loading in this phase.

```rust
// Illustrative contracts: refine to idiomatic Rust and async-safe types.
trait CatalogRepository { /* roots, files, scan sessions, jobs */ }
trait InventorySource { /* list immediate children, stat entry */ }
trait JobProcessor { /* process bounded unit with checkpoint/cancellation */ }
trait ResourceMonitor { /* resource snapshot and change notification */ }
trait SchedulingPolicy { /* may_run(job, snapshot) -> decision/reason */ }
trait ModelProvider { /* probe endpoint, list model IDs */ }
trait DocumentProcessor { /* reserved: probe type, extraction capabilities */ }
trait SourceOpener { /* validate/open/reveal recorded source */ }
```

**Minimum built-in adapters:** local filesystem inventory, SQLite catalog, OS system probe, Ollama endpoint probe, OpenAI-compatible probe, and native source opener. Document processing is interface-only in Phase 0; do **not** create a dummy PDF parser that claims to extract content.

Expose typed IPC commands (exact spelling may differ, keep stable DTOs):

```text
get_app_status()
get_device_snapshot()
choose_data_root(path) / open_data_root(path)
list_source_roots() / add_source_root(path) / remove_source_root(id)
start_inventory(source_root_id) / pause_job(job_id) / resume_job(job_id) / cancel_job(job_id)
get_inventory_status() / list_documents(query, root_id?, type?, page, page_size)
open_original(file_id) / reveal_original(file_id)
list_provider_profiles() / save_provider_profile(profile)
test_provider_connection(profile_id_or_candidate)
```

Use paginated queries; never send tens of thousands of file records to the frontend in one response. Emit throttled, typed progress events (e.g., at most several per second) instead of excessive IPC traffic. Errors contain a stable code, safe message, and optional recovery action. Do not include raw secret tokens, extracted content, or unredacted user file paths in diagnostics by default.

**Security boundary:** The renderer may *request* actions through the command API but must not have unrestricted filesystem, shell, SQL, or HTTP access. Tauri capabilities are narrow. Validate IDs/paths on the Rust side; only open existing cataloged documents within authorized roots. Never implement source opening through string-built shell commands.

## 10. Model-provider connection spike

This is a **connection test**, not inference or RAG.

- Ollama: allow configuring its URL; test the documented tags/models endpoint with timeout and parse returned model identifiers.
- OpenAI-compatible: test model-list endpoint (`/models` under the configured API base path) with optional Bearer credential; be clear that some providers may disable listing and require manual model IDs in a later phase.
- Make HTTP requests only after explicit user action; no startup probes that leak identifiers or documents. Limit response size, request count, and timeout; allow cancellation.
- Store API keys in OS credential storage, referenced from the provider record. If secure storage is unavailable, explain it and allow session-only credentials; never silently downgrade to unencrypted disk storage.
- Require TLS for public internet URLs; allow local loopback HTTP and opt-in LAN HTTP with visible security explanation. Reject accidental embedded credentials in URL.
- Only display success after a valid endpoint response. Distinguish network failure, authentication failure, protocol mismatch, and no models available.
- Do not submit any library content in connection tests.

## 11. Test fixture collection

Create a reproducible, copyright-free fixture generator and README. Include:

- 200–1,000 synthetic PDF/EPUB-named test files across many nested directories (a subset may contain valid minimal PDF/EPUB structures; do not require text extraction).
- Duplicate filenames in different folders, mixed case, whitespace, Unicode, long filenames, filenames containing punctuation.
- Nested folders; empty folders; an intentionally unreadable directory where platform permissions permit; unsupported file types.
- Symlink cycle and symlink outside selected root (platform-gated if symlink creation is restricted).
- 1,000+ files for pagination and scan progress; a bigger synthetic set (e.g., 30,000 lightweight files) for performance/recovery benchmarking.
- A deliberately missing/unmounted source path; read-only data destination; injectably low disk space; simulated power transitions.

**Do not commit personal ebooks/papers or downloaded copyrighted content to the repository.** Fixtures must not mutate any user collection.

## 12. Required automated tests

| ID | Scenario | Expected result |
| --- | --- | --- |
| P0-01 | Fresh first run | Onboarding shown; no forced account/model setup |
| P0-02 | Create/reopen Atlas data root | Root UUID/schema persist; same catalog reopens |
| P0-03 | Unsupported newer schema | Safe read-only error/refusal to write, no corruption |
| P0-04 | Two app instances on one root | Second writer blocked with useful message |
| P0-05 | Inventory PDFs/EPUBs in nested tree | Correct unique file instances; no source file writes |
| P0-06 | Scan overlapping roots | Documented, deterministic handling; no runaway duplicate enumeration |
| P0-07 | Symlink loop/outside-root | Skipped; scan terminates without escape |
| P0-08 | Restart after forced process kill | Incomplete work resumes; no duplicate catalog rows |
| P0-09 | User pauses scan | Job stops cooperatively and remains paused across restart |
| P0-10 | Battery drops to ≤20% | Bulk inventory pauses; safe resume when policy permits |
| P0-11 | Unknown battery state | Conservative behavior; truthful Unknown indicator |
| P0-12 | Low data-root disk space | New write batches pause; no DB corruption |
| P0-13 | Source root disappears | Catalog remains; not all items marked missing |
| P0-14 | Atlas data drive disappears | No new empty library silently created; clear recovery UI |
| P0-15 | Basic file browse/search | Paginated, filtered library while scan progresses |
| P0-16 | Open/reveal a cataloged PDF | Correct OS action; no shell injection; offline file handled |
| P0-17 | Ollama probe (mock service) | Model IDs displayed; no document upload |
| P0-18 | OpenAI-compatible probe with key | Secret not logged/exported or stored in SQLite |
| P0-19 | Remote HTTP endpoint | Blocked unless allowed local/LAN exception |
| P0-20 | Data-root nested in source | Atlas-generated subtree excluded |
| P0-21 | App exits while scanning | No persistent background scanner survives by default |
| P0-22 | Invalid, unreadable, or huge path | Useful recoverable error; app doesn't hang |

Use injected fake `ResourceMonitor`/`InventorySource`/HTTP clients for deterministic resource and error tests. Include database migration tests and assertion that source-file hashes/stat metadata do not change during inventory. Negative cases must be tested, not just the happy path.

## 13. Benchmarks and hardware validation

Record performance, don't invent guarantees. At minimum measure:

- Fresh launch to usable window; folder selection to first visible catalog rows.
- Inventory throughput, CPU %, peak process RAM, disk growth, and application responsiveness on a 1,000-file fixture and a 30,000-file lightweight fixture.
- AC vs simulated/on-device battery operation; effect of pause/resume and concurrent browsing.
- Durable recovery after forced termination.
- SQLite DB/WAL size and cold/warm paginated browse/query latency.
- macOS arm64 device first; Windows x64 and Linux x64 CI and smoke evidence before complete sign-off.

Use a provisional target of **responsive common UI operations while scanning** (95th-percentile service command latency under ~250 ms on a modern SSD-equipped computer for 1,000-file fixture), then report actual numbers, test hardware, and methodology. This is a working performance target, **not a published minimum system requirement**. Revisit the initial inventory throttle based on measurements, not assumptions.

## 14. Cross-platform packaging, CI and release evidence

- CI matrix: macOS arm64 runner where available, Windows x64, Linux x64. Run Rust format/lint/tests, TypeScript typecheck/lint/tests, package builds, and package/artifact upload.
- Build development distributions: macOS `.app`/`.dmg`, Windows NSIS `.exe` setup installer, Linux AppImage and/or `.deb`. A literal portable single executable is **not required** in Phase 0.
- Public signing/notarization may be postponed, but document expected OS security prompts and do not claim the artifacts are production-ready.
- Smoke-test launching, choosing directories, scanning, opening source, pausing/resuming, and restarting **on all three OSes**. If automated E2E is unreliable on a given platform, provide and execute a manual smoke checklist with evidence.
- Linux webview/system dependencies must be documented; do not claim universal compatibility with every distribution.
- Artifacts must not download or depend on Python, Docker, SQLite server, Ollama, or an LLM to start the basic app.
- Commit reproducible lockfiles and an initial dependency/license report; flag GPL/AGPL or commercial requirements, bundled binary obligations, and unresolved licensing choices.

## 15. Implementation backlog: ordered vertical slices

Implement and verify **one slice at a time**. The agent should build/rerun tests after each slice, then proceed. Never claim later slices are done because scaffolding compiles.

| Slice | Deliverable | Verification gate |
| --- | --- | --- |
| 0.1 | Tauri + Svelte repo, theme, app shell, dev docs | Launch on primary development OS; lint/tests pass |
| 0.2 | Atlas root chooser, manifest, settings, SQLite migration and root lock | Reopen after restart; newer schema/second writer rejected safely |
| 0.3 | Native source chooser and authorized source-root records | Add/remove roots; originals unchanged |
| 0.4 | System probes, preflight and low-space policy | Fake probe tests cover AC/battery/unknown/low space |
| 0.5 | Durable directory frontier and single-worker inventory scheduler | Kill/restart/pause/resume tests pass |
| 0.6 | Library list, paging/filtering, open/reveal | Useful partial results during scan; offline file error handled |
| 0.7 | Provider settings and connection probes | Ollama/OpenAI-compatible mock probes; key safety tests |
| 0.8 | CI, packaging, cross-platform smoke and benchmark report | Evidence for macOS, Windows, Linux; limitations documented |

Each slice includes: code, automated tests, short README/doc update, explicit demo steps, and a brief note of any deviations from this specification. Suggested checkpoints/commits use names `phase0/slice-01` through `phase0/slice-08`, but do not assume a remote repository exists.

## 16. Acceptance checklist for the product owner

- [ ] Starts without terminal setup, Docker, Python, account, or LLM.
- [ ] Can choose an Atlas data directory and reopen the same catalog after restart.
- [ ] Can add multiple existing book/PDF roots; no source files altered or moved.
- [ ] Can browse progressively discovered PDFs and EPUBs with filename search.
- [ ] Can open an original source in the OS default application.
- [ ] Can pause, restart, and resume after a forced application kill without duplication.
- [ ] Gives understandable resource/battery/low-space guidance; never monopolizes machine by default.
- [ ] Generated persistent Atlas metadata and indexes reside beneath the selected data root, with documented exceptions.
- [ ] Can test a local Ollama endpoint without uploading documents; optional credential handling is secure.
- [ ] Does not expose unprotected HTTP server/SQL/shell/filesystem access to frontend code.
- [ ] Has recorded test, build, license, and platform-smoke evidence.
- [ ] Clearly distinguishes functionality implemented now from future AI and cloud features.

## 17. Constraints for the coding agent

1. **Read this entire spec and Atlas Product Specification v0.2 first.** Treat the parent spec as product direction and this spec as the Phase 0 scope/acceptance contract.
2. Work incrementally, beginning with Slice 0.1. Deliver working code rather than architecture-only commentary. Prefer maintained existing libraries over custom replacements.
3. If implementation conflicts with a stated invariant (read-only originals, single data root, recoverable jobs, no mandatory AI), preserve the invariant and document the tradeoff.
4. Do not silently substitute Python backend, Electron, standalone server/daemon, arbitrary runtime plugin system, cloud account, or externally managed database.
5. Don't overbuild future phases. Define interfaces and reserve layout where necessary, but implement only what Phase 0 exercises.
6. Validate inputs on the Rust side. Treat source filenames and model endpoint responses as untrusted data. Never execute file paths in a shell.
7. Use explicit migrations and structured error codes. Prefer deterministic fixtures/mocks to flaky real hardware tests, supplemented by physical-platform smoke tests.
8. Run code formatting, static checks, and tests before claiming each slice is done. Report commands executed and outcomes.
9. Never claim a Windows/Linux/macOS build was tested unless it was actually run/tested there. Clearly label unverifiable steps.
10. End each slice report with: implemented, tests passed, manual verification, known issues, and next slice.

## 18. Kickoff instructions for the coding agent

**First requested task:** Implement **Slice 0.1** only: scaffold a Tauri 2 + Svelte/TypeScript app named Atlas, set up the Rust module skeleton and frontend typed command facade, implement the polished but minimal Welcome/Home/Library/Settings shell with system theme support, establish formatting/typechecking/test commands, write `docs/architecture.md` and an initial ADR explaining the no-server, one-data-root approach, and configure the basic OS build matrix. Stub APIs must return explicit `NotImplemented` errors instead of invented sample library data. After the slice passes, summarize code and verification and provide the exact command to run the app locally.

Do **not** move straight to ingestion, embedding, OCR, or cloud functionality without passing the prior slice gates.

---

### Primary reference documentation (verify current versions at implementation time)

- Tauri v2 project setup: <https://v2.tauri.app/start/create-project/>
- Tauri v2 plugin catalog and capabilities: <https://v2.tauri.app/plugin/>
- Tauri native folder dialog: <https://v2.tauri.app/plugin/dialog/>
- Tauri opener: <https://v2.tauri.app/plugin/opener/>
- Tauri testing: <https://v2.tauri.app/develop/tests/>
- Tauri Windows installers: <https://v2.tauri.app/distribute/windows-installer/>
- SQLite: <https://www.sqlite.org/docs.html>
- Rust system information: <https://docs.rs/sysinfo/latest/sysinfo/>
