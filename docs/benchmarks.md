# Benchmarks (Phase 0)

These are measurements, not guarantees (phase0-spec §13). They describe
the computers listed with each table, on the dates given. CI runners are
shared virtual machines with noisy neighbours; use their numbers to compare
operating systems and spot regressions, not as absolute speeds.

## Method

**Harness:** `src-tauri/benches/inventory.rs`, run in the release profile
through `scripts/bench.mjs` (`just bench`, `just bench-full`). It
catalogs fixtures through the real services: the local filesystem source,
SQLite catalog, OS volume inspector and resource monitor, and the default
`InventoryConfig` (100-entry batches, 2 s policy re-checks, 250 ms progress
events). Only the power reading is simulated, so AC and battery runs are
repeatable.

**Fixtures:** made by `scripts/generate-fixtures.mjs`:

- **1k**: 1,006 catalogable PDF/EPUB files plus unsupported files,
  symlinks, empty and locked folders (1,365 entries).
- **30k**: 30,006 catalogable files, including one flat folder of 20,000
  (spec §11).

Files are tiny stubs, so these runs measure directory listing and catalog
writes, not reading file contents (Phase 0 never opens contents). Each run
starts from a new, empty data folder. The fixture is freshly generated or
recently listed, so the filesystem cache is warm. A first scan of a cold
external drive will be slower.

**What is measured** in each run:

| Measure | How |
| --- | --- |
| Wall time, files/s | From adding the folder until the scheduler is idle |
| Scan CPU % of one core | Process CPU time (sysinfo) over wall time, minus the harness's own baseline measured just before |
| Peak RSS | Resident memory sampled every 100 ms (a sampled peak, not a true high-water mark) |
| First rows | Time from adding the folder until the first page query returns at least one row (the UI step "folder selection to first visible rows" without rendering) |
| Browse while scanning | A second thread issues the browse mix 5 times a second during the scan; per-command latency |
| Warm queries | 60 commands from the browse mix on the finished library |
| Status query | 30 `get_inventory_status` calls on the finished library |
| Cold first page | First-page query right after closing and reopening the library |
| DB + WAL | `catalog.sqlite` and `-wal` sizes after the scan and after closing |
| Pause/resume | Pause at about 30% of files. Files written in the next 500 ms (must be 0), then resume-to-next-write time |
| Recovery | A child process is killed mid-scan, then the library is reopened. Files at kill, time to complete, duplicate rows (must be 0) |

**The browse mix:** first page by name; search "volume 12"; page 20 of 50
sorted by date; type and folder filter with search "book 1"; a search with
no matches; and the indexing status. These are the commands the Library
and status bar send.

**The harness doesn't measure itself.** It follows the runner's progress
events rather than polling the catalog. (An earlier version polled the
status every 100 ms. At 30k files that poll cost about half a core, which
is how the status-count problem below was found.)

<!-- results:start -->
<!-- results:end -->
