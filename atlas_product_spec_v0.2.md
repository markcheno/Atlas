# Atlas — Product Specification v0.2

**Status:** Working product specification; updated 2026-11-07  
**Audience:** Product owner and implementation agents  
**Initial platform:** macOS, Windows, Linux  
**Deployment strategy:** Local-first desktop; future open-source self-hosted and paid hosted/cloud service  
**Primary user:** An individual with thousands to tens of thousands of PDFs, EPUBs, books, and research papers in partially organized directories
**Version changes:** Extension points; resource-aware onboarding and scheduling; unified configurable data root; open-source dependency policy

## 1. Vision

Atlas is a **personal AI librarian** that makes a lifetime of collected knowledge accessible again. It helps people rediscover forgotten materials, find information by meaning, explore unexpected connections, and save discoveries for future interests or projects.

It is **not** primarily a bibliographic manager, folder organizer, general note-taking app, or autonomous research agent.

**Core promise:** *Collect freely. Organize minimally. Retrieve intelligently.*

### Design principles

1. **Curiosity first:** Begin with the user's present interest or question, not a complicated catalog.
2. **Effortless onboarding:** Point to one or more folders; Atlas starts working without manual tagging, Docker, Python, a terminal, or an account.
3. **Read-only sources:** Never rename, move, modify, or delete original documents without explicit permission.
4. **Useful without an LLM:** Browse, keyword search, metadata, bookmarks, and source opening all work with AI disabled.
5. **Local-first and private:** Documents stay local unless the user explicitly enables external processing or sync.
6. **Source-grounded intelligence:** Research answers cite identifiable documents and precise pages or sections whenever possible.
7. **Knowledge portability:** Notes, trails, references, and annotations have exportable formats; Obsidian is optional.
8. **Progressive complexity:** Minimal default interface; advanced controls only when requested.
9. **Shared core, different deployments:** Keep local/self-hosted and cloud deployment paths compatible without building cloud infrastructure prematurely.
10. **Respect device resources:** Never allow automatic bulk indexing to monopolize CPU, memory, storage, battery, or network resources.
11. **One Atlas data home:** Keep Atlas-owned persistent artifacts under one user-selected data root, apart from original sources and OS-protected secrets.
12. **Reuse before reinventing:** Adopt maintained open-source libraries and standard protocols where they solve the problem; invest custom code in Atlas workflows and integration.
13. **Extension-ready, not extension-dependent:** Core modules have explicit interfaces; third-party runtime plugins and a marketplace are not MVP requirements.

## 2. Who it is for

Initial use case: one person with hundreds of GB and tens of thousands of documents organized into mixed directories such as `Books/`, `PDFs/`, and `Purchased/`, with variable metadata, duplicates, and some scans. They may work across astronomy, engineering, mathematics, science, history, software, and other interests.

Eventually serve researchers, lifelong learners, hobbyists, and enthusiasts with similarly large private collections.

### Jobs to be done

- **Find what I vaguely remember:** “I collected something about harmonic-drive mechanisms years ago.”
- **Find what is relevant now:** “What do I own about detecting meteors in noisy video?”
- **Rediscover what I forgot:** Surface useful but previously overlooked books and papers.
- **Make connections:** Compare perspectives or identify overlapping concepts across otherwise unrelated subjects.
- **Resume curiosity:** Return to an earlier exploration without reconstructing its searches and sources.
- **Retain insight:** Save excerpts, citations, annotations, or summaries to Atlas and optionally export to Obsidian.

## 3. User experience and information architecture

The desktop application should feel calm, fast, and polished, inspired by Apple-like restraint without depending on Apple frameworks. Start with a prominent search field and a small number of meaningful views. Respect native OS conventions, keyboard navigation, accessibility, and light/dark modes.

### Core views

**Home — “What are you curious about?”**  
A single search/ask input; a few current interests or projects; optional “From your library” rediscovery suggestions; recent research trails. No dashboard clutter or mandatory taxonomy.

**Search / Research**  
One query field with two explicit modes: *Find sources* and *Ask Atlas*. Hybrid full-text and semantic retrieval. Results show title, source type, location/page, matching passage, and why it may be relevant. AI answers contain clickable citations, uncertainty markers when needed, and access to original sources. Filtering by folder, author, year, type, topic is available but not intrusive.

**Document detail**  
Title, authors, source path(s), abstract/description, extracted sections, related documents, bookmarks, and source viewer/open-original action. A PDF or EPUB reader may initially delegate to the system default app; embedded reading is a later enhancement. Original page links should survive path moves where possible.

**Interests, Projects and Research Trails**  
An *interest* is informal and indefinite; a *project* is goal-oriented; a *trail* is a sequence of questions, sources, annotations, and findings from an exploration session. Atlas records research trails and allows intentional saving, editing, deletion, and export; it does not automatically convert every interest into a project.

**Library**  
Traditional browsing by source folder, documents, authors, types, and inferred topics, but not the default home experience. Expose duplicate groups, extraction status, and indexing issues here.

**Settings**  
Simple top level: Library Folders, AI Models, Privacy, Storage, **Performance & Battery**, Appearance. Advanced: model roles, indexing strategy, provider timeouts, OCR, worker limits, background schedule, diagnostics, extension management (later), API, export/import. Show *used space* and *available space* in Storage. Avoid raw internals in default views.

### Onboarding flow

1. Welcome: Explain local-first behavior and let user continue without registration.
2. Choose one or more existing **source folders**; Atlas requests read-only access and never moves them.
3. Select or accept a sensible default **Atlas data location**; check free space, write access, and the likelihood that the indexes will fit. Explain clearly that this stores *generated Atlas data*, not copies of all source documents.
4. Run a short **device check** (memory, CPU/architecture, disk, whether running on battery, optionally GPU capability). Apply a quiet, conservative background-work profile automatically; avoid prompting users for technical values.
5. Begin **throttled background inventory**. Library browsing and basic search become usable progressively; initial indexing must not block the UI or run a GPU-heavy model without consent.
6. Offer optional AI setup: **Run AI locally** or **Connect an AI provider**. The user may skip AI entirely; recommendations use device information but do not require understanding model parameters.
7. Present a few example queries and clear progress status (inventoried / text extracted / semantically indexed / OCR required). Display ordinary-language statuses, e.g., “Indexing paused while on battery.”

No forced manual tags, folder relocation, required cloud connection, or mandatory Obsidian installation.

## 4. Model providers and configuration — first-class feature

Atlas must be **model-agnostic**. Users can connect to a local or remote inference endpoint via a provider preset or custom URL. Model settings are not hard-coded into search or document storage.

### Model roles

| Role | Purpose | Needed when |
| --- | --- | --- |
| Chat / research | Explain, summarize, synthesize, answer | User selects Ask Atlas |
| Embedding | Semantic retrieval and related items | Semantic search enabled |
| Reranker | Improve candidate ranking | Optional; later |
| Vision / OCR assist | Interpret complex scanned pages and images | Optional; later |

The first release supports separate **chat** and **embedding** configurations. The other roles are extension points, not initial features.

### Provider presets

- Ollama (local or remote endpoint, native adapter)
- LM Studio (OpenAI-compatible endpoint)
- OpenAI (native adapter)
- Anthropic (native adapter)
- Custom OpenAI-compatible HTTP endpoint
- Additional adapters (for example Gemini, OpenRouter, llama.cpp/vLLM servers) after capability validation

**Important:** Not all providers use the same protocol, so a universal “base URL” setting is not sufficient on its own. The provider adapter handles authentication, requests, streaming, and capability checks. Custom compatible providers implement the supported compatibility subset.

### Settings fields

- Provider name and type
- Base URL (prefilled for presets, editable)
- API key / credentials if needed, entered securely
- Model identifier (discover available IDs when possible, or manually enter)
- Role assignment: chat and/or embeddings
- Connection test; model-capability test (chat versus embeddings)
- Privacy indicator: local / LAN / external
- Optional request limits, timeouts, context length, token budget, and per-provider cost reporting

**Secrets:** Store API keys in OS-backed credential storage or a secure secret manager, not plaintext settings, frontend local storage, logs, or exports. In future hosted deployments credentials remain server-side, scoped to an account. Require HTTPS for remote services unless a knowledgeable user explicitly opts into a trusted local-network connection.

### Recommendations

Provide a small, maintained recommendation catalog based on hardware, **memory headroom after Atlas and other apps**, CPU/GPU availability, available disk, battery/power source, privacy preference, role, installed runtimes, model capabilities, and optional local benchmarks. Present **recommended / faster / higher-quality** choices, not a giant leaderboard. Recommendations are advice, never mandatory.

**Initial candidates for a 64 GB Apple Silicon Mac (benchmark before declaring defaults):**

- Chat: `qwen3.5:9b` for everyday tasks; `qwen3.5:27b` for richer synthesis.
- Embeddings: `qwen3-embedding:0.6b` to begin; `qwen3-embedding:4b` as a quality benchmark.
- Cloud: an explicitly chosen provider/model for optional complex research.

**Switching behavior:** Changing chat models does not trigger reindexing. Changing embedding model, output dimensions, or certain text preprocessing settings creates a **new versioned vector index**; Atlas continues serving the old index while the new one builds, then switches after validation.

**Request limits:** Each remote provider adapter supports configured requests/minute, concurrency, timeouts, and optional cost ceilings; honor server `429`/retry-after signals with bounded exponential backoff. Cloud processing and remote embedding of an entire corpus are distinct, explicitly opt-in operations.

**Privacy:** Never upload the collection merely because a cloud model is configured. Disclose what excerpts/metadata will leave the device before the first external AI request. Background catalog enrichment stays local unless separately enabled. Allow users to disable all external AI.

**Failure behavior:** If a provider is unavailable, keyword/bibliographic search and library features continue to work. Show actionable errors without breaking the application.

## 5. Library and intelligence capabilities

### Ingestion

- Read-only recursive discovery of PDF/EPUB files in user-selected directories.
- Filesystem watchers plus periodic reconciliation for changes, moves, and deletions.
- Content fingerprints and file-instance IDs to detect exact duplicates and preserve links across moves.
- Extract metadata, text, headings, page/section references, and thumbnails where practical.
- Ordinary digital PDFs and EPUBs first; scanned-document OCR progressively; per-file processing status.
- Durable, resumable and **priority-aware** ingestion jobs with errors, retries, cancellation, checkpointing, and per-document processing state.
- Rate-limited processing that yields to interactive browsing, user-requested searches, and AI questions; obey battery and low-space policies in Sections 8–10.
- Never prevent use of already indexed material while processing continues.

### Data model (conceptual)

- **Workspace / owner** — organizational boundary for future cloud isolation.
- **Source root** — user-authorized folder or remote source.
- **File instance** — path, hash, size, type, metadata, filesystem status.
- **Logical document** — book/paper/work (may have multiple file instances or editions).
- **Section / passage** — extracted content with page, location, and extraction version.
- **Topic / relationship** — inferred or manually maintained labels and relationships with provenance.
- **Interest / project / collection** — user-controlled grouping and context.
- **Research trail / event** — searches, selected sources, answers, annotations, bookmarks.
- **Model profile / index version** — provider configuration and reproducible index provenance.
- **Background job / resource profile** — durable task state, checkpoints, memory/CPU/power budgets, scheduling policy.
- **Extension manifest / capability grant** — identity and version of bundled or future external implementation, permissions granted, and compatibility metadata.

Not every conceptual object requires a separate database table in v0.1. Keep user-created knowledge separate from derived/rebuildable indexes.

### Retrieval and research

- Lexical search (SQLite FTS5) plus semantic search (embedded vector index).
- Hierarchical retrieval: document summaries first, deeper passage candidates when needed.
- Optional reranking and evidence selection.
- Citation-backed answers referencing original document and page/section.
- Never invent citations; distinguish extracted source text from LLM interpretation.
- Prevent document text from being treated as instructions to the LLM.
- Small, repeatable retrieval evaluation corpus with user-known questions.

## 6. Cross-platform application and packaging

**Preferred direction to validate:** Tauri 2 + Svelte/TypeScript UI + Rust application core. Core services expose typed interfaces and a versioned API contract; the desktop product does not depend on a separately installed web server.

- macOS: signed/notarized `.app` distributed via `.dmg`.
- Windows: single `.exe` installer; portable single-file variant only if dependencies permit.
- Linux: AppImage and/or standard distro packages.

**Distribution objective:** One ordinary installer/download, not necessarily a literal single binary. No compulsory Docker, Python interpreter, database daemon, or bundled multi-GB model. Extra ML/OCR runtimes can be optional, managed components. Any bundled helper/sidecar must be installed and updated as part of the application, without separately requiring end-user command-line setup. Do not promise a portable one-file executable until dependent native libraries and code signing are validated.

The extraction layer is modular. Evaluate native document libraries and an optional **bundled sidecar/helper** for complex parsing/OCR; package all required runtime components or install optional features through the UI. The initial runtime needs **no separate Python installation**. Test macOS arm64, Windows x64, and Linux x64 first; Linux arm64 and Windows arm64 can follow based on demand. Validate minimum supported OS versions and required Linux webview libraries through packaging tests rather than assuming universal binary compatibility. Assess licenses early, especially AGPL/GPL dependencies, given the intended open-source plus commercial hosting model.

### API strategy

Keep app services behind stable versioned contracts from the beginning. Tauri can call local core services directly; a separately exposed authenticated HTTP API becomes an opt-in feature or future cloud service. API endpoints must never bypass per-workspace authorization. Avoid exposing a broad unauthenticated loopback server by default.

## 7. Hardware support, minimum specifications, and preflight checks

Atlas must distinguish **basic library features** from **optional on-device AI**. A user with a modest computer should still be able to inventory, browse, and search documents using exact/full-text search or remote API models.

### Initial support targets (provisional; validate with benchmarks)

| Capability | Minimum target | Recommended target | Notes |
| --- | --- | --- | --- |
| OS/architecture | Supported 64-bit desktop OS, 2 CPU cores | Modern 4+ core desktop/laptop | macOS/Windows/Linux version matrix established in packaging tests |
| RAM for catalog, reading and keyword search | **8 GB** | **16 GB+** | Large libraries benefit from more RAM; this is a target, not yet a measured guarantee |
| RAM for optional local small LLMs | Model-specific; generally **16 GB+** for many quantized small models | **32–64 GB** for larger models and long context | Never block library use because local LLM requirements are unmet |
| GPU/NPU | Not required | Supported hardware acceleration helpful | AI provider/runtime determines GPU support |
| Free storage | Enough for application plus estimated Atlas data; start only when an initial safety reserve exists | Fast SSD/NVMe, ample space beyond estimated index | The collection itself can live on another internal or external drive |
| Network | None required for core local features | Optional for model downloads or cloud APIs | Offline use must remain functional |

**The application must not advertise precise hardware minimums as tested facts until Phase 0 performance tests establish them.** The first release should publish a validated compatibility matrix by OS version, architecture, library size, and enabled AI features.

### First-run and continuous checks

- Check physical/available RAM, processor architecture and logical cores, free disk space on the **selected Atlas data volume** (separately from source volumes), directory permissions, and whether the chosen data root is reachable.
- Detect AC versus battery, approximate charge level, and memory/thermal pressure **when reported by the OS**. Degrade gracefully if any reading is unavailable or unreliable. A desktop machine with no battery must not be treated as a laptop on battery.
- Estimate likely indexing expansion by sampling representative files (for example, 50–200, bounded by time) and observing extracted text volume, document count, OCR prevalence, and vector dimensions. Display a broad **estimate/range**, not false precision.
- Allow a catalog-only mode when storage, memory, model availability, or battery state would make a deeper index impractical.
- The default low-space guard pauses **new heavy work** if remaining space is below `max(2 GiB, min(10 GiB, 2% of volume capacity))`; use more conservative headroom for migrations and large individual jobs. Expose a configurable override with clear risk information.
- Provide user-readable recommendations: “This computer can comfortably catalog and search your books. Local AI may be slower; you can connect a service instead.”
- Recheck resources before large jobs, after drive reattachment/wake, and periodically during ingestion; never rely on one initial hardware test.

## 8. One configurable Atlas data root

All **Atlas-created persistent application data** should live together under a single user-defined directory (the **Atlas Data Root**). This is separate from read-only document source roots. The application chooses an OS-appropriate default but allows changing it during setup or later, including to an external SSD. Avoid writing generated indexes beside source documents.

### Proposed logical layout (implementation filenames may evolve)

```text
<ATLAS_DATA_ROOT>/
  atlas.json                    # root ID, layout and schema version; no secrets
  config/
    settings.json               # portable preferences; no API keys
  library/
    catalog.sqlite              # documents, notes, trails, jobs, FTS and metadata
    # SQLite WAL/SHM reside here while in use
  content/
    extracted/                 # derived, location-addressable text
  indexes/
    vectors/                   # per-model/version semantic indexes
  assets/
    thumbnails/                # cover and preview images
  cache/
    previews/                  # rebuildable temporary presentation cache
  jobs/
    checkpoints/               # resumable ingestion job artifacts
  models/                      # Atlas-managed models, if used; optional
  exports/                     # user-requested Markdown/JSON exports
  logs/                        # privacy-redacted application diagnostics
  tmp/                         # staging/scratch; safe to clean after recovery
  backups/                     # verified snapshots, if enabled
```

**Exceptions, documented explicitly:** OS keychain/credential vault entries cannot be portable files in this directory; a tiny OS-managed locator may store the selected root path so Atlas can start; OS installer and webview/system caches may be outside the root where the platform requires it. A separately installed Ollama/LM Studio stores its own models wherever *that runtime* is configured; Atlas must clearly identify externally managed storage rather than implying it is inside Atlas's root.

### Data-root requirements

- Unique persistent root identity and schema version; schema migrations must be transactional or recoverable, and an older app must refuse unsafe writes to a newer schema.
- All file paths inside the data root are resolved relative to the root wherever practical. Source file paths are referenced, never automatically copied into the root.
- If the data root is inside a watched source folder, exclude it recursively from ingestion; prevent directory symlink cycles and duplicate traversal.
- Prevent concurrent writers to the same data root, using an instance lock and clear recovery options; allow multiple unrelated data roots/workspaces in future designs.
- **Move Atlas Data** is a first-class operation: pause jobs, verify destination space, take a consistent database snapshot, copy/verify files, atomically switch the locator, and roll back safely on failure.
- Separate **irreplaceable user-authored data** (notes, annotations, trails, projects, saved citations, preferences) from rebuildable caches and semantic indexes. Provide backup/export and restore verification; do not count indexes as a substitute for backups.
- Expose storage usage by category, safe cache cleanup, and optional quotas/retention. Do not silently delete user notes, exports, or backups to recover disk space.
- Clarify that moving the data root to another computer does not automatically make original documents available: source roots may require re-linking and permissions.
- Local metadata/indexes are normally stored as ordinary files, protected by OS permissions and recommended full-disk encryption. Future application-level encryption and encrypted sync require explicit design; do not misrepresent local data as end-to-end encrypted.

## 9. Background jobs, throttling, and battery protection

Indexing a huge library must be **low priority by default**. Atlas may take days to finish a very large initial collection, but it must remain responsive and must not unexpectedly drain a laptop battery.

### Queue and priority model

- **Interactive / high priority:** open a document, search, browse, retrieve sources, user-requested answer.
- **On-demand / medium priority:** index the specific book selected for research, create a thumbnail, extract a requested chapter.
- **Background / low priority:** discover files, hash duplicates, extract full text, OCR, generate embeddings, compute relationships, optimize indexes.
- Persistent job records with idempotency keys, checkpoint/resume, cancellation, bounded retries, exponential backoff, per-file failure isolation, and worker heartbeats. Make restart/retry safe; processing one corrupt document must not block the queue.
- Initial conservative limits for **background** jobs: one inventory traversal, up to two lightweight extractors, one OCR worker, and one embedding worker (subject to model memory and actual benchmark results). Do not run all categories at maximum simultaneously; the scheduler enforces a shared CPU/RAM/IO budget.
- Batch writes to reduce contention; cap filesystem reads, writes, and network calls; yield to interactive activity. An automatic slowdown must also apply when the machine is busy with *other* applications.

### Default resource policy

| Condition | Automatic policy |
| --- | --- |
| Plugged in, machine idle/lightly used | Run background jobs under conservative CPU/memory/IO budget |
| User actively searching/reading or running local AI | Yield or sharply reduce bulk work; preserve UI responsiveness |
| On battery | **Pause heavy OCR, bulk embeddings, and long-running AI enrichment**; permit lightweight browsing and limited inventory if inexpensive |
| Battery low (suggested threshold 20%) | Pause bulk indexing entirely; continue core library features |
| High memory pressure, thermal warning, or high sustained system load | Reduce concurrency or pause; respect OS indicators where supported |
| Low disk space or data volume disconnected | Immediately stop new writes/jobs; preserve completed work; explain how to resume |
| Device sleeps/restarts or app exits | Safely checkpoint and stop; resume after relaunch subject to policy |
| Remote API rate-limits or budget exceeded | Back off or pause that provider's jobs without blocking local search |

Expose a small **Indexing** status (e.g. “12,400 of 30,000 files cataloged · Paused on battery”), with **Pause**, **Resume**, and an optional **Index now / While charging / Scheduled** choice. Advanced users may set concurrency, CPU budgets, quiet hours, model memory caps, and network/provider limits. Never claim a fixed percentage for unbounded discovery; distinguish discovered files from processed files.

**Process lifecycle:** In the personal desktop edition, closing the app must stop CPU-heavy tasks by default. Running a persistent background service after closing Atlas is opt-in and a future capability—not a hidden daemon. The app should resume safely next time.

### Testable safety outcomes

- With default settings, bulk OCR/embedding stops when switching to battery and resumes when policy allows, without losing successful work.
- Search and source opening remain responsive during a large ingest; benchmark against explicit responsiveness targets from Phase 0.
- Killing the app mid-document does not corrupt the catalog or mark unfinished work as successfully indexed.
- A low-space transition produces a controlled pause, not a failed database write or corrupted index.
- API `429` and network loss trigger bounded retries without repeated paid requests or duplicated logical work.

## 10. Extensibility and plugin architecture

**Recommendation:** Design for plugins, but initially ship a **modular monolith** using trusted in-process implementations and, where needed, bundled helper executables. An installable third-party plugin marketplace is explicitly deferred. Tauri's native plugin framework covers Tauri/runtime integrations; it should not be mistaken for a safe arbitrary-plugin sandbox.

### Interfaces to establish early

| Extension interface | Examples | Initial behavior |
| --- | --- | --- |
| `DocumentExtractor` | PDF, EPUB, OCR, future HTML/Office | Bundled implementations selected by file signature/capability |
| `AIProvider` | Ollama, LM Studio, hosted APIs | Built-in connection profiles and role-aware capability checks |
| `EmbeddingBackend` / `Reranker` | Local embeddings, remote embeddings, optional reranking | Swap provider without changing catalog model; version indexes |
| `SearchBackend` | SQLite FTS5, embedded vector index | Atlas orchestrates common retrieval contract |
| `SourceConnector` | Local folders, future cloud drives, web clipping | Local directories only in MVP |
| `Exporter` / `Importer` | Markdown, JSON, Obsidian links | Built-in Markdown/JSON initially |
| `StorageBackend` | Local files/SQLite, future hosted object stores/SQL | Backend adapter; not arbitrary user-installed DB driver |

Interfaces must be versioned and pass typed data with stable IDs, provenance, explicit errors, progress/cancellation tokens, and resource estimates. External implementations should receive a **minimum necessary capability grant**: selected files, limited HTTP destinations, specific API operations, and no blanket access to the data root or API keys.

### Runtime extension policy (later)

- Prefer signed/verified packages and an explicit install/enable/disable/revoke lifecycle with per-workspace grants.
- Investigate **out-of-process RPC or WASM/WASI** execution for untrusted extensions. Avoid arbitrary dynamically linked Rust libraries as a portable, stable plugin ABI.
- Treat plugin code as potentially malicious: limit filesystem/network privileges and resource usage; isolate crashes; provide compatibility and permission prompts before activation.
- Document third-party plugin API versioning and security review prior to opening the ecosystem.

**Out of scope for MVP:** Plugin marketplace, community plugin upload, arbitrary scripting, and executing plugins fetched from the internet.

## 11. Open-source reuse and dependency governance

Atlas should build differentiated value in the **experience, data model, orchestration, and discovery workflows**, not in reinventing general infrastructure.

| Need | Reuse / evaluate | Decision gate |
| --- | --- | --- |
| Desktop shell, installer, permissions | Tauri 2, platform webview APIs | Cross-platform packaging, signing and security tests |
| Catalog, full-text search | SQLite + FTS5 | Benchmark multi-GB extracted text and phrase search |
| PDF text, layout, rendering | PDFium with appropriate Rust binding, or a packaged parser helper | Text positions, citations, crash containment, redistribution license |
| EPUB text/metadata | Maintained EPUB parser and standard ZIP/XML tooling | Real-world malformed EPUB fixtures |
| Scanned-document OCR | Tesseract (optional component) | Quality, languages, performance and packaging |
| Complex PDF/table extraction | Docling or comparable existing tool (optional bundled helper) | Accuracy, dependency size, cross-platform installers |
| Vector search | Embedded LanceDB or comparable open engine | Index size, rebuild/swap, recall, portability |
| LLM inference / embeddings | Ollama, LM Studio, MLX-compatible runtimes; API services | Use provider APIs; do not write model runtime |
| Hardware/power telemetry | OS APIs and maintained Rust crates (for example `sysinfo`, battery telemetry libraries) | Verify each OS, fallbacks and measurement overhead |

**Dependency policy:** Prefer actively maintained, permissively licensed components (MIT/Apache-2.0/BSD) when fit is comparable, but do not reject a useful dependency solely because it is copyleft. Document the exact license and distribution/hosting obligations before selecting GPL/AGPL or proprietary/commercial dual-license components. Produce an SBOM, license/NOTICE inventory, pinned or reproducible dependency manifests, vulnerability scans, and upgrade policy. Evaluate transitive dependencies and bundled native libraries—not only top-level Rust/JS packages. Retain freedom to replace individual processors when quality or license constraints change.

**Build-versus-buy rule:** Adopt an existing library if it meets fidelity, packaging, license, maintenance, privacy, and performance needs. Write Atlas-specific code only where existing tools do not supply the product behavior. Start with a reference implementation, then benchmark it against real documents; avoid speculative rewrites in Rust for tasks already solved well elsewhere.

## 12. Deployment, privacy, and product editions

**Atlas Community:** Fully useful, open-source, self-hosted local app with library ingestion, search, discovery, AI-provider connections, research trails, notes, export, and API/integration capability. Do not cripple essential discovery features to encourage upgrades.

**Atlas Cloud (future):** Paid managed hosting for automatic sync, encrypted storage, remote access, hosted processing options, account management, backups, and multi-device convenience. A hybrid mode may sync knowledge metadata and trails while keeping full documents and indexing on the desktop.

Distinguish transport/at-rest encryption from true end-to-end encryption. A cloud service cannot arbitrarily index plaintext that its operator cannot decrypt; encrypted sync and hosted processing are separate, explicit trust modes. Build threat modeling, key recovery, account isolation, quotas, and audit requirements before deploying any public service.

Do not sync entire document collections to mobile devices by default. Sync small user-authored data first, metadata selectively, and original documents on demand. Sync/conflict resolution is a later phase, not part of MVP.

## 13. Implementation phases and completion criteria

### Phase 0 — Product UX and architectural spike

Deliver screen flows, core data contracts, buildable Tauri skeleton, a 200–1,000-file representative test set, packaging spike for each OS, and ingestion/library benchmarks. Add a data-root layout spike, resource telemetry/power-state detection on each platform, conservative background scheduler prototype, and a basic extractor/provider adapter contract. Record dependency licenses before committing. Decide on extraction/vector libraries after practical tests and licensing review.

**Done when:** App launches on macOS, Windows, and Linux; adds a directory; persists a catalog entry entirely under the selected data root; opens a source file; and connects to a test local model endpoint. The app can pause/resume a durable indexing job, detect missing data volume/low storage, and report battery state where supported.

### Phase 1 — Useful local library

Add folder scanning, metadata, extraction, deduplication, indexes, browse/open, exact/full-text search, configurable data-root management, device preflight checks, and resilient, battery-aware progress reporting. AI setup remains optional.

**Done when:** The user can find forgotten files reliably; rescans do not duplicate unchanged work or modify originals; and indexing pauses safely on battery, low space, or shutdown.

### Phase 2 — AI librarian

Add provider presets/configuration, embeddings, semantic discovery, related documents, question-answering, and source citations. Support per-role model choice and privacy indicators.

**Done when:** A vague natural-language query retrieves known relevant documents, and an answer reliably opens its supporting source locations.

### Phase 3 — Knowledge workspace

Add interests, projects, bookmarks, saved findings, research trails, rediscovery suggestions, and Markdown export/optional Obsidian linking.

**Done when:** A prior research session can be resumed after weeks or months without manually recreating it.

### Phase 4 — Full-collection readiness

Optimize at tens-of-thousands-of-files scale, add prioritized OCR, index versioning, backups, integrity checks, recoverable data-root moves, diagnostics, performance/battery profiles, and robust incremental updates.

**Done when:** Hundreds of GB can be indexed progressively and recovered after failures without losing user notes.

### Phase 5 — Secure sync and multi-device

Add optional account/device identities, end-to-end encrypted sync for supported data, selective remote access, mobile-friendly clients, offline caching, and explicit merge/recovery behavior.

**Done when:** Research trails and bookmarks synchronize safely, while full documents remain local unless opted in.

### Phase 6 — Hosted commercial service

Build multi-tenant hosting, authenticated public APIs, permission enforcement, billing, quotas, managed model services, encrypted storage options, monitoring, and security review.

**Done when:** Multiple independent users have provably isolated private libraries and understandable control over storage, processing, and costs.

## 14. MVP exclusions

No mandatory cloud login, full-library uploads, automatic file reorganization, public document sharing, real-time collaborative editing, sophisticated graph editors, agents modifying source materials, required Obsidian integration, large built-in note editor, native iOS/Android apps, third-party plugin marketplace, or required background daemon.

## 15. Next specification artifacts

1. **UX flows and wireframes** — Home, Search/Research, Document Detail, Library, Settings.
2. **Data model and API contracts** — Stable IDs, source links, trails, index versions, provider configs.
3. **Ingestion / retrieval technical design** — Benchmark extraction, plugin interface contracts, resource-aware scheduling, and search quality before locking packages.
4. **Phase 0/1 coding-agent implementation backlog** — Small vertical slices, tests, cross-platform build process, acceptance criteria.

## 16. Open decisions to validate

- Native Rust-only parsing versus optional bundled document-processing helper.
- Embedded vector indexing engine and persistence format.
- Exact PDF/EPUB reader scope in early releases.
- Degree of automatic research-trail capture and user controls.
- Licensing choice for the open-source codebase and document-processing dependencies.
- Mechanism for a user to access the local Atlas library remotely before cloud sync exists.
- Exact minimum supported OS versions and measured baseline resource requirements on each desktop platform.
- Default background CPU/IO policy that balances indexing speed and laptop comfort on typical hardware.
- Packaging and update strategy for optional OCR/complex PDF helpers without requiring users to install Python.
- Storage layout stability, encrypted-at-rest options, and portability across machines with differently mounted source directories.
- Scope and security model for third-party plugins once there's demonstrated demand.

**Product north star:** Someone should be able to install Atlas, point it at a lifelong document collection, ask about something they're curious about, find useful sources, and return to that exploration later—with very little manual organization.

---

### References for technical validation

- Tauri plugin design: <https://v2.tauri.app/develop/plugins/>
- Tauri capabilities/permissions: <https://tauri.app/security/capabilities/>
- Tauri bundled executables: <https://v2.tauri.app/develop/sidecar/>
- Rust cross-platform system telemetry: <https://docs.rs/sysinfo/latest/sysinfo/>
- Rust cross-platform battery information: <https://docs.rs/starship-battery/latest/starship_battery/>
- PDFium Rust binding/packaging alternatives: <https://docs.rs/pdfium-render/latest/pdfium_render/>
- Tesseract OCR and licensing: <https://tesseract-ocr.github.io/tessdoc/>
- SQLite FTS5 full-text index: <https://www.sqlite.org/fts5.html>

- Tauri distribution: <https://v2.tauri.app/distribute/>
- LM Studio OpenAI-compatible endpoints: <https://lmstudio.ai/docs/developer/openai-compat>
- Ollama model catalog: <https://ollama.com/library/qwen3.5> and <https://ollama.com/library/qwen3-embedding>
- PyMuPDF licensing: <https://pymupdf.io/licensing>
- OpenAI API key best practices: <https://developers.openai.com/api/docs/guides/api-keys>
