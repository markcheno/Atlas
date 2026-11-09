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

## Machines (2026-11-08)

| Label | Machine | Run |
| --- | --- | --- |
| **M5 Pro** | MacBook Pro (Mac17,9), Apple M5 Pro, 18 cores, 64 GB, internal SSD, macOS 27.0.1. On battery power (the harness simulates the power reading, so the policy is the same either way). Nothing else running | `just bench-full` |
| **CI macOS** | GitHub `macos-latest`: Apple M1 (virtual), 3 cores, 7 GB, macOS 26.7.2 | CI run [42487239937](https://github.com/markcheno/Atlas/actions/runs/42487239937) |
| **CI Windows** | GitHub `windows-latest`: AMD EPYC 7763, 4 cores, 16 GB, Windows Server 2025, Defender on | same run |
| **CI Linux** | GitHub `ubuntu-24.04`: Intel Xeon 6973P-C, 4 cores, 16 GB, ext4 | same run |

Each CI job uploads its raw JSON and tables as the `bench-<OS>` artifact
(kept 90 days) and adds the tables to the run summary.

## Results at the policy rate (plugged in, 500 entries/s)

| Fixture | Measure | M5 Pro | CI macOS | CI Windows | CI Linux |
| --- | --- | ---: | ---: | ---: | ---: |
| 1k | Wall time | 3.3 s | 3.3 s | 3.6 s | 3.2 s |
| 1k | First rows | 41 ms | 68 ms | 33 ms | 25 ms |
| 1k | Browse p95 while scanning | 3.1 ms | 2.7 ms | 3.5 ms | 2.0 ms |
| 1k | Peak RSS | 17 MiB | 15 MiB | 14 MiB | 9 MiB |
| 1k | Catalog (DB + WAL, closed) | 1.3 MiB | 1.2 MiB | 1.3 MiB | 1.2 MiB |
| 30k | Wall time | 79 s | 80 s | 99 s | 79 s |
| 30k | Scan CPU, % of one core ¹ | 18.7 | 15.5 | 19.6 | 16.8 |
| 30k | First rows | 45 ms | 98 ms | 33 ms | 29 ms |
| 30k | Browse p95 while scanning (worst command) | 41 ms | 125 ms | 44 ms | 35 ms |
| 30k | Browse max while scanning | 45 ms | 220 ms | 85 ms | 38 ms |
| 30k | Warm queries p95 | 22 ms | 88 ms | 43 ms | 33 ms |
| 30k | Status p95 while scanning | 12 ms | 18 ms | 18 ms | 10 ms |
| 30k | Cold first page after reopening | 1.0 ms | 3.6 ms | 6.1 ms | 2.9 ms |
| 30k | Peak RSS | 20 MiB | 18 MiB | 17 MiB | 14 MiB |
| 30k | Catalog (DB + WAL, closed) | 29.6 MiB | 27.1 MiB | 28.4 MiB | 27.0 MiB |
| 30k | Kill mid-scan: resume to complete | 1.3 s | 3.2 s | 39.8 s | 4.2 s |
| both | Duplicate rows after kill and resume | 0 | 0 | 0 | 0 |
| both | Files written after pause | 0 | 0 | 0 | 0 |

¹ Includes the browse thread's queries (five browse commands a second
during the whole scan), which cost more as the catalog grows. The sweep
below runs without browsing; there the scan alone costs 7.2% of a core at
this rate.

The catalog grows by about 1 MiB per 1,000 files (with all search keys and
indexes). On battery (simulated 50%) the slowed rate of 100 entries/s
catalogs the 1k fixture (1,365 entries) in 13.7 s on every machine, at
under 3% of a core.

Unthrottled, the 30k fixture takes 1.2–2.0 s on the M5 Pro, 4.6 s on CI
macOS, 5.9 s on CI Linux, and 47 s on CI Windows. Windows is limited by the
filesystem (NTFS and Defender scanning each listing), not by Atlas's CPU,
which stays below 20% of a core there.

## Provisional targets (§13)

| Target | Result |
| --- | --- |
| Browse p95 below 250 ms on the 1k fixture while scanning | **Met** on all four machines (2.0–3.5 ms) |
| The same on the 30k fixture (not a Phase 0 target) | Met: worst command p95 35–125 ms, worst single call 220 ms (CI macOS) |
| Launch to a usable window | 1.5 s (M5 Pro, release, manual); 1.0 s (CI Linux) and 2.6–2.9 s (CI Windows) in the debug smoke test |
| Folder selection to first visible rows | 0.4–0.6 s in the CI smoke test (includes clicking through onboarding); 25–98 ms for the catalog alone |

None of these are guarantees; the CI figures come from shared machines.

## The throttle (§13 revisit)

Slice 0.5 started at 100 entries/s plugged in (20 slowed), a cautious
guess. The rule
for Slice 0.8: choose the highest rate at which the scan alone stays well
under 25% of one core on the M5 Pro, and browse latency stays under the
targets above on every CI runner. The 30k sweep on the M5 Pro, without
browsing:

| Rate (entries/s) | Wall time | Scan CPU, % of one core | Peak RSS |
| ---: | ---: | ---: | ---: |
| unthrottled | 1.2 s | 77.8 | 18 MiB |
| 1,000 | 30.4 s | 11.2 | 20 MiB |
| **500** | **60.8 s** | **7.2** | **20 MiB** |
| 250 | 121.5 s | 4.6 | 20 MiB |
| 100 | 303.7 s | 2.5 | 20 MiB |

**Decision: 500 plugged in, 100 slowed** (one fifth). At 500 the scan
costs about 7% of a core on the M5 Pro and 15–20% on the 3–4 core CI
machines, including browsing, with browse latency well within target. 1,000
would halve the time again but leaves less headroom on slower machines.
The rates are in `src-tauri/src/services/scheduler/policy.rs`; ADR 0004 has
the amendment.

## Problems the benchmarks found

- **Status counts were slow at 30k files** (about 20 ms per call), and the
  first harness polled them often enough to cost half a core. A covering
  index (migration 0006) brought them to about 4 ms on the M5 Pro.
- **On Windows the status waited for the writer.** It went through the
  single writer thread, so during scans it queued behind catalog writes:
  p95 368 ms (1k) and 919 ms (30k), with one call taking 10 s. Status is now
  read on the pooled read-only connections: 0.9 ms and 18 ms in the same
  scenarios.
- **Pause at the batch boundary.** A batch listed before a pause could
  still commit after it. Commits now check that the session is still
  running (ADR 0005); every run shows 0 files written after the pause.

## Known limitation: resuming inside a very large folder

Resuming a pause that happened inside the 20,000-file folder takes 16.6 to
18.5 s before the first new file appears, at the policy rate, on every
machine (0.3–1.4 s unthrottled). A folder is the unit of resumption
(ADR 0005): it is listed again from the start, and the entries already
written before the pause are replayed through the same throttle. Paused
about 9,000 entries in, that is 9,000 ÷ 500 entries/s ≈ 18 s. The status
shows the files already cataloged meanwhile. Charging less for replayed
entries, or resuming mid-folder, is left for a later phase.

## Regression check: Slice 1.1 (general jobs)

Slice 1.1 moved the scan onto the general job runner
([ADR 0009](adr/0009-jobs-and-extraction.md)) without changing its logic.
`just bench` ran on the M5 Pro before (on `main`) and after, on the same
fixtures, back to back (2026-11-08):

| 30k, plugged in | Before | After |
| --- | ---: | ---: |
| Files/s at the policy rate | 378.3 | 378.1 |
| Scan CPU, % of a core | 17.9 | 17.8 |
| Peak RSS, MiB | 20.2 | 19.9 |
| Browse p95 while scanning, ms | 33.49 | 33.57 |
| Status p95 while scanning, ms | 11.28 | 12.39 |
| Files/s unthrottled | 14,939 | 14,443 |
| Files written after a pause | 0 | 0 |
| Duplicate rows after a kill | 0 | 0 |

The 1k and battery scenarios match as closely (73.4 and 73.6 files/s on
battery). Every difference is within the spread between repeated runs of
the same build; the unthrottled 30k run lasts only 2 s.

## Extraction worker start

Slice 1.2 measures how long `atlas --extract-worker` takes from spawn to its
`hello` line, which includes loading PDFium and extracting the built-in
two-page self-test PDF (`scripts/worker-hello.mjs`, 10 starts; median of
starts 2–10). The spec switches to a separate sidecar binary above about
300 ms on any OS (phase1-spec §7.2); every median is under 30 ms, so the
worker stays a mode of the app executable (2026-11-08).

| Build | Where | First start | Median | CI run |
| --- | --- | ---: | ---: | --- |
| macOS debug | M5 Pro, `target/debug` | 20 ms | 17 ms | local |
| macOS debug `.app` | M5 Pro | 18 ms | 15 ms | local |
| macOS release `.app` | M5 Pro | 241 ms | 15 ms | local |
| macOS release, DMG copy | M5 Pro | 321 ms | 18 ms | local |
| macOS debug `.app` | CI arm64 | 57 ms | 18 ms | [42531070487](https://github.com/markcheno/Atlas/actions/runs/42531070487) |
| macOS release `.app` | CI arm64 | 471 ms | 22 ms | 42531070487 |
| Windows debug | CI x64 | 135 ms | 19 ms | [42531636951](https://github.com/markcheno/Atlas/actions/runs/42531636951) |
| Windows, NSIS per-user install | CI x64 | 69 ms | 21 ms | 42531636951 |
| Linux debug | CI x64 | 45 ms | 28 ms | 42531070487 |
| Linux `.deb` layout | CI x64 | 155 ms | 23 ms | 42531070487 |
| Linux AppImage | CI x64 | 1,673 ms | 1,206 ms | 42531070487 |

- The first start of a newly signed or copied macOS binary takes 0.2–0.5 s
  once while the OS checks its signature; later starts are 15–22 ms. A
  sidecar would be a second binary paying the same one-time cost.
- The AppImage row measures `APPIMAGE_EXTRACT_AND_RUN=1`, which unpacks the
  86 MiB image on every start because the runner has no FUSE. The app
  spawns the worker from its already-mounted image (`current_exe`), so that
  cost doesn't apply to it.
- The spike's "3 ms" for pdfium never loaded the library (ADR 0008,
  amended).

## Text extraction, first measurement (Slice 1.3)

The extraction benchmark proper (`benches/extraction.rs`, three OSes, duty
cycles, catalog growth) is Slice 1.7. For a first number, the worker alone
read the 1k text fixture (`just fixtures <dir> 1000 0 20`: 1,006 files,
6,668 PDF pages plus EPUB chapters) one file at a time, hashing then
extracting each, with no duty cycle, on the M5 Pro (2026-11-09, debug Rust,
release PDFium):

| Files | Passages (PDF pages and EPUB pieces) | Characters | Time | Files/s | Passages/s |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1,006 | 9,372 | 20.8 million | 7.2 s | 141 | 1,310 |

At the Run duty cycle (50% busy) that is about 650 pages a second, above
the provisional 200 pages/s target (phase1-spec §13). The spike measured
pdfium alone at 1,294 pages/s on its corpus.

Reading a Phase 0 library on upgrade: the copied 30,006-file stub library
(identical PDF stubs, so one document) was hashed and linked in about a
minute in a debug build at the default duty cycle; the 3,334 stub "EPUBs"
are correctly `malformed` (not real zips).

## Search (Slice 1.4)

`cargo bench --bench search` (`just bench-search`) catalogs and reads the
30k text fixture (`--text-kb 20`) unthrottled through the real services
and worker, querying a mix of searches every 200 ms while text is being
read, then 20 rounds idle, then the first query of each kind after
reopening. M5 Pro, release build, 2026-11-09:

| Files | Passages | Text | Read | First searchable | Index | Catalog | Index rebuild |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 30,006 | 293,070 | 622 M chars | 147 s | 3.3 s | 248 MB | 1.4 GB | 7.5 s |

| `search_documents` p50 / p95 / max ms | While reading | Idle |
| --- | ---: | ---: |
| All queries | 59 / 152 / 211 | 105 / 175 / 178 |
| Single term (`orbital`) | 77 / 124 / 136 | – |
| Phrase (`"quiet harbour lights"`) | 69 / 121 / 133 | – |
| Prefix (`resonan*`) | 117 / 195 / 211 | – |
| Rare term (a marker phrase) | 33 / 41 / 46 | – |
| Common term (`river`) | 72 / 130 / 143 | – |
| Any words (`orbital banana`) | 79 / 126 / 144 | – |
| File name (`book 000042`) | 33 / 42 / 48 | – |

Cold, the first query of each kind after reopening took 31–177 ms. P1-19
(p95 under 250 ms while reading, on the reference machine) passes; CI
reports the same table per OS without asserting it. On the 1k text fixture
every query kind stays under 7 ms.

The catalog holds the text and its index: 1.4 GB for 622 MB of text,
2.3×, above the provisional 1.5× (phase1-spec §13); see ADR 0010. Reading
in the release build ran at 204 files/s and 1,990 passages/s, unthrottled.

<!-- results:end -->
