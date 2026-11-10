# Benchmarks

These are measurements, not guarantees (phase0-spec §13, phase1-spec §13,
phase2-spec §13). They describe
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

## Resuming inside a very large folder

Until Slice 1.7, resuming a pause that happened inside the 20,000-file
folder took 16.6 to 18.5 s before the first new file appeared, at the
policy rate, on every machine (0.3–1.4 s unthrottled). A folder is the unit
of resumption (ADR 0005): it is listed again from the start, and the
entries already written before the pause went through the same throttle
again. Paused about 9,000 entries in, that is 9,000 ÷ 500 entries/s ≈ 18 s.

Slice 1.7 charges those replayed entries a twentieth of an operation (ADR
0004, ADR 0005): the scan's upsert leaves a row it already wrote unchanged
untouched and counts it. Same benchmark (`--pause` on the 30k fixture,
paused about 30% in), policy rate (P1-20 asks for under 3 s):

| Machine | Before (Slice 0.8) | Slice 1.7 | Files written after pause |
| --- | ---: | ---: | ---: |
| M5 Pro (2026-11-09) | 16.6–18.5 s | 0.95 s | 0 |
| macOS arm64 runner ([42634733190](https://github.com/markcheno/Atlas/actions/runs/42634733190)) | 16.6–18.5 s | 0.99 s | 0 |
| Windows x64 runner (same run) | 16.6–18.5 s | 0.86 s | 0 |
| Linux x64 runner (same run) | 16.6–18.5 s | 0.93 s | 0 |

Unthrottled, the same resume takes 87–261 ms on the runners. The
remaining second is the 450 operations the replayed 9,000 entries still
cost, plus listing them again. `tests/inventory.rs` (P1-20) checks the
same effect at a fixed rate of 400 entries/s on 4,000 files: the next new
file must arrive in under half the 4 s that replaying at full cost takes.
With the discount turned off, the test fails at 4.0 s.

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

The extraction benchmark proper is in
[Text extraction (Slice 1.7)](#text-extraction-slice-17). For a first number, the worker alone
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
(p95 under 250 ms while reading, on the reference machine) passes. CI runs
the search benchmark on the 1k text fixture on each OS and reports it
without asserting: the 30k text fixture took over 40 minutes on a Windows
runner, so the 30k measurement is a local one (`just bench-search`). On the 1k text fixture
every query kind stays under 7 ms.

The catalog holds the text and its index: 1.4 GB for 622 MB of text,
2.3×, above the provisional 1.5× (phase1-spec §13); see ADR 0010. Reading
in the release build ran at 204 files/s and 1,990 passages/s, unthrottled.


## Duplicates and storage (Slice 1.6)

The search benchmark's fixtures now include identical copies (`--copies
2000` at 30k, `--copies 100` at 1k), and after the idle search rounds it
times the first page of `list_duplicates` (20 rounds) and
`get_storage_usage` (5 rounds). M5 Pro, release build, 2026-11-09:

| Fixture | Groups | `list_duplicates` p50 / p95 / max ms | `get_storage_usage` p50 / p95 / max ms |
| --- | ---: | ---: | ---: |
| 1k text + 100 copies | 101 | 0.6 / 0.7 / 0.7 | 8.5 / 8.6 / 8.6 |
| 30k text + 2,000 copies | 2,001 | 44 / 53 / 53 | 281 / 294 / 294 |

Both are well inside the 250 ms browsing target for a list, and the
storage measurement runs on its own connection outside the workspace
lock, so 0.3 s at 30k needs no caching (the panel shows "Measuring…").
The same run's search (32,006 files with the copies) had p95 196 ms while
reading and 179 ms idle, still under P1-19's 250 ms.

**Where the catalog's size goes.** `dbstat` splits the 30k catalog
(1,409 MiB) into the stored text, 1,057 MiB of pages for 594 MiB of
characters; the full-text index, 237 MiB; and everything else, 115 MiB.
So the 2.3× growth recorded in Slice 1.4 comes mostly from the
`passages` table, not from the index: rows of about 3,000 characters
fill SQLite's 4 KiB pages poorly. That points Slice 1.7's growth work at
the page size or at compressing passages, not at the index.

## Text extraction (Slice 1.7)

`cargo bench --bench extraction` (`just bench-extraction`) catalogs a text
fixture unthrottled, then reads it through the real job, supervisor and
worker: at the Run duty cycle (50% busy, on AC), at the Slow one (20%, a
simulated battery at 50%) and unthrottled. It samples the app and the
worker separately, checkpoints the catalog and splits it with `dbstat`,
and then kills a child app mid-read and finishes the job after reopening.
Pages are PDF pages; passages add the EPUB pieces. Growth is against the
text's stored bytes.

**M5 Pro, 30k text fixture** (`30k-text-copies`: 32,006 files, 266,598 PDF
pages, 293,068 passages, 594 MiB of text), release build, 2026-11-09:

| Duty cycle | Read s | Files/s | Pages/s | First searchable s | App CPU % of a core | Worker CPU % | App / worker peak RSS MiB | Worker starts | Peak WAL MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Run (50%) | 350.4 | 91.3 | 761 | 0.30 | 28.2 | 26.0 | 45 / 21 | 1 | 26 |
| Unthrottled | 133.8 | 239.3 | 1,993 | 0.02 | 44.0 | 60.6 | 47 / 20 | 1 | 29 |

The policy said Run for the whole Run pass. The Run duty cycle reads 761
pages a second, almost four times the provisional 200 pages/s
(phase1-spec §13), so one worker is enough for Phase 1 (ADR 0009).
The app's share is the catalog writer storing passages and keeping the
full-text index in step. The end-of-pass index merge takes under a
millisecond; the whole job ends as soon as the last file is read.

**Kill mid-read** (30k, 6,408 documents read when the child was killed):
no partial documents, no worker left running, no duplicate documents,
nothing left waiting; reading the rest after reopening took 106 s
unthrottled.

**The duty cycle's effect on wall time.** A 50% duty cycle should take
twice as long as unthrottled, 20% five times. With a rest after every file
it took far longer: the files take milliseconds, and each millisecond
sleep cost more than it rested (Windows rounds timers to about 15.6 ms;
caches go cold and cores clock down, so on the M5 Pro the worker used 30%
more CPU time for the same 1k fixture at Run, and 2.2× at Slow). Run took
2.5× as long as unthrottled on the M5 Pro and 2.0–5.6× on the runners, with
the policy at Run throughout. Rests now add up and are taken once they
reach 100 ms (`REST_QUANTUM`), the same busy fraction in bursts of about
100 ms:

| M5 Pro | Before | Rests of at least 100 ms |
| --- | ---: | ---: |
| 1k, Run ÷ unthrottled wall time | 11.3 s ÷ 4.5 s = 2.5× | 7.7 s ÷ 3.9 s = 2.0× |
| 1k, Slow ÷ unthrottled | 47.5 s ÷ 4.5 s = 10.6× | 34.2 s ÷ 3.9 s = 8.8× |
| 30k, Run | 518.8 s (514 pages/s) | 350.4 s (761 pages/s) |

Slow stays above 5×: at 20% the bursts are 25 ms, short enough for the
cores to clock down between them, which on battery is the point.

**Catalog growth** at 30k, new catalog (16 KiB pages):

| Page size | Text stored | Catalog | Text pages | Index | Other | Catalog ÷ text |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 KiB (Slice 1.6, same fixture) | 594 MiB | 1,409 MiB | 1,057 MiB | 237 MiB | 115 MiB | 2.37 |
| 16 KiB (Slice 1.7) | 594 MiB | 963 MiB | 680 MiB | 236 MiB | 47 MiB | 1.62 |

On the 1k text fixture, back to back on the M5 Pro: 44.3 MiB (2.24×) with
4 KiB pages and 33.3 MiB (1.68×) with 16 KiB; reading unthrottled was 2–3%
slower with 16 KiB pages over repeated runs (2,455–2,687 against
2,588–2,751 passages/s), within the spread between first and later runs.
The 1.62× is still above the provisional 1.5×; the difference is the
full-text index, which exact phrases need ([ADR 0010](adr/0010-full-text-search.md)).
Catalogs created before Slice 1.7 keep 4 KiB pages.

**Problems this benchmark found.** The end of every pass waited 45–48 s
at 30k files (0 s at 1k): the Slice 1.6 query for documents due to be read
again was quadratic in a root's files, holding the catalog writer. It now
starts from the marked documents ([ADR 0011](adr/0011-re-reading-without-losing-search.md));
an unthrottled 30k read went from 192 s to 134 s. The first harness also
read the library's status every 100 ms, which at 30k files cost the app
about 20% of a core of its own; it now reads it every 500 ms once text is
searchable.

**CI runners, 1k text fixture** (1,106 files, 6,668 PDF pages, 9,322
passages, 19.8 MiB of text), release build, [run 42651988551](https://github.com/markcheno/Atlas/actions/runs/42651988551),
2026-11-09. The policy said Run, or on battery for the Slow pass,
throughout:

| Runner | Duty cycle | Read s | Pages/s | App / worker CPU % | App / worker peak RSS MiB | Catalog ÷ text |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| macOS arm64 (M1, 3 vCPU) | Run | 30.5 | 219 | 14.2 / 19.9 | 27 / 19 | 1.68 |
| | Slow | 89.7 | 74 | 6.1 / 6.8 | 31 / 19 | 1.68 |
| | Unthrottled | 13.3 | 503 | 34.4 / 56.7 | 31 / 19 | 1.68 |
| Windows x64 (Xeon, 4 vCPU) | Run | 41.3 | 162 | 14.7 / 15.5 | 24 / 16 | 1.68 |
| | Slow | 158.3 | 42 | 7.7 / 4.3 | 24 / 16 | 1.68 |
| | Unthrottled | 19.7 | 339 | 22.2 / 31.1 | 20 / 16 | 1.68 |
| Linux x64 (EPYC, 4 vCPU) | Run | 18.7 | 356 | 13.7 / 28.3 | 17 / 44 | 1.67 |
| | Slow | 47.8 | 139 | 6.5 / 11.4 | 18 / 44 | 1.68 |
| | Unthrottled | 9.7 | 691 | 23.4 / 54.8 | 18 / 44 | 1.68 |

One worker start per pass everywhere, the first passage searchable within
0.5 s, and the WAL peaked at 16–18 MiB. Run takes 1.9–2.3× as long as
unthrottled on the runners (2.0–5.6× before rests were pooled). Kill
mid-read on every runner: 222–232 documents read, no partial documents,
the worker gone within 55 ms, no duplicates, nothing left waiting.

## Phase 1 summary (Slice 1.8)

The close-out's numbers, on the final code (version 0.2.0), against the
provisional targets of phase1-spec §13. The reference machine is the M5
Pro (`just bench`, release build, 2026-11-09); the runners are CI run
[42689464772](https://github.com/markcheno/Atlas/actions/runs/42689464772)
(1k fixtures) and the evidence run
[42685716386](https://github.com/markcheno/Atlas/actions/runs/42685716386)
(the 30k text fixture, unthrottled, every file required to be read).

| Measure | M5 Pro | macOS arm64 runner | Windows x64 runner | Linux x64 runner | Target |
| --- | ---: | ---: | ---: | ---: | --- |
| Search p95 while reading (P1-19) | 158 ms (30k) | 15 ms (1k) | 20 ms (1k) | 16 ms (1k) | under about 250 ms at 30k |
| Reading at the Run duty cycle | 777 pages/s (30k) | 237 pages/s (1k) | 168 pages/s (1k) | 356 pages/s (1k) | at least 200 on the reference machine |
| 30k text fixture read, unthrottled | 135 s, 32,006 of 32,006 | 445 s, all | 1,145 s, all | 431 s, all | 100% of well-formed files |
| Catalog ÷ stored text | 1.62 (30k) | 1.62 (30k) | 1.62 (30k) | 1.62 (30k) | 1.5; 1.62 accepted |
| Resume inside the 20k folder (P1-20) | 0.94 s | 0.95 s | 0.87 s | 0.93 s | under 3 s |
| Kill mid-scan: duplicate rows | 0 | 0 | 0 | 0 | 0 |
| Kill mid-read: duplicates, partial documents, workers left | 0, 0, 0 | 0, 0, 0 | 0, 0, 0 | 0, 0, 0 | 0 |

On the M5 Pro the 30k search run read 32,006 files with 293,068 passages
while querying: p95 158 ms (p50 61 ms, max 286 ms, a single any-words
query), idle p95 176 ms, the first query of each kind after reopening
34–173 ms; the index is 236 MiB and rebuilds in 6.7 s. The catalog is
965 MiB, against 1.4 GB in Slice 1.4 before 16 KiB pages. Library ›
Duplicates' first page takes 33 ms at p95 over 2,001 groups, and Settings
› Storage 122 ms. Cataloging is unchanged since Phase 0: 483 files/s at
the policy rate on 30k, browse p95 under 37 ms while scanning.

## Vector index (Slice 2.2)

The index alone (`benches/vectors.rs`, `just bench-vectors <size>`):

- **Data:** synthetic clustered unit vectors of 1,024 dimensions (a Gaussian
  mixture, as the Slice 0.9 spike's), stored as Atlas stores them.
- **Catalog rows:** f16 BLOBs in a 16 KiB-page catalog, with one document
  per 100 chunks.
- **Index:** built in memory, then saved, loaded and rebuilt through
  `open_index`.
- **Recall@10** is against brute force over the same stored vectors (200
  queries).
- **Latency:** 1,000 queries on one thread.
- **Filters:** each document belongs to one of 8 folders of skewed size, so
  filtered searches run at about 40%, 5% and 0.5% selectivity.
- **Index settings:** cosine, f16, connectivity 16, expansion 128 / 128.
- **Runs:** the reference machine (M5 Pro, 18 threads, release build,
  2026-11-09), and the CI runners at 100,000 in run
  [42724704393](https://github.com/markcheno/Atlas/actions/runs/42724704393).

| Machine, vectors | Build s | Save s | Open → first answer s | Rebuild from catalog s | File ÷ raw f32 | Index memory MiB | Catalog bytes per vector |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| M5 Pro, 100,000 | 3.1 | 0.28 | 0.15 | 3.5 | 0.536 | 273 | 2,534 |
| M5 Pro, 300,000 | 11.5 | 0.83 | 0.44 | 12.7 | 0.536 | 1,139 | 2,530 |
| M5 Pro, 1,000,000 | 49.3 | 2.69 | 1.53 | 53.6 | 0.536 | 2,299 | 2,529 |
| macOS arm64 runner (3 vCPU), 100,000 | 30.2 | 0.92 | 0.30 | 32.3 | 0.536 | – | 2,534 |
| Windows x64 runner (4 vCPU), 100,000 | 19.7 | 3.05 | 0.28 | 22.0 | 0.536 | – | 2,534 |
| Linux x64 runner (4 vCPU), 100,000 | 18.0 | 0.52 | 0.24 | 18.6 | 0.536 | – | 2,534 |

| Recall@10, p95 ms | Unfiltered | One folder, about 40% | About 5% | About 0.5% |
| --- | ---: | ---: | ---: | ---: |
| M5 Pro, 100,000 | 0.992, 0.36 | 0.997, 0.46 | 0.997, 4.6 | 0.998, 39 |
| M5 Pro, 300,000 | 0.990, 0.47 | 0.994, 0.61 | 0.997, 4.8 | 0.993, 48 |
| M5 Pro, 1,000,000 | 0.960, 0.82 | 0.984, 1.1 | 0.989, 6.7 | 0.994, 58 |
| macOS arm64 runner, 100,000 | 0.993, 2.2 | 0.997, 1.8 | 1.000, 15 | 0.996, 144 |
| Windows x64 runner, 100,000 | 0.993, 0.67 | 0.998, 0.93 | 0.999, 8.1 | 1.000, 81 |
| Linux x64 runner, 100,000 | 0.993, 0.55 | 0.998, 0.70 | 1.000, 6.6 | 0.994, 64 |

What this shows:

- **ADR 0008's targets hold at a million.** Recall@10 is 0.960 (≥ 0.95),
  p95 0.82 ms (≤ 100 ms), one folder at 5% 0.989 and 6.7 ms (≥ 0.90,
  ≤ 150 ms), and open to first answer 1.53 s (≤ 2 s). These match the
  spike's numbers (0.963, 0.84 ms, 0.988 and 6.8 ms), now through
  Atlas's own code.
- **Opening includes the BLAKE3 check** of the whole 2.1 GB file before
  usearch sees it. That is why it takes longer than the spike's 0.50 s
  memory-mapped view.
- **The narrowest folder is the slow case.** Graph search slows as a filter
  narrows: 58 ms at 0.5% on the reference machine, 144 ms on the 3-vCPU
  macOS runner. Slice 2.4 adds an exact search for such scopes.
- **Saves take 2.7 s at a million.** Searches continue during a save; only
  adds wait (ADR 0013).
- **Catalog: 2,529 bytes per vector with 16 KiB pages.** That is about 1%
  over the provisional 2.5 KB (phase2-spec §13): the vector is 2,048 bytes,
  and its hash and model id appear twice. In a catalog created before
  Slice 1.7 (4 KiB pages; `--page-size 4096`) it is 4,294 bytes, because a
  page holds only one vector. Slice 2.3 decides what to do about that.
- **The query plan matters.** Before `CROSS JOIN` pinned the rebuild
  query to chunk-id order, each batch sorted the whole join, and a rebuild
  at 100,000 took 9.4 s instead of 3.5 s.
- **Build cost:** usearch, numkong and cxx add about 8 s to a clean release
  build on the reference machine (cargo `--timings`: numkong 4.5 s, usearch
  2.2 s, cxx 1.1 s).

## Embedding on oMLX (Slice 2.3)

The embedding job's own counters (`jobs.embedded_chunks`, `sent_chars`,
`service_ms`) from the product owner's run on the reference machine
(2026-11-09, debug build). The service was oMLX with
`Qwen3-Embedding-0.6B-8bit` at 1,024 dimensions. The library was the spike
corpus plus 1,000 generated books: 1,020 documents with text, 22.4 million
characters.

| Measure | Result |
| --- | ---: |
| Chunks sent (1,200 characters, 150 overlap) | 26,453 (all distinct; 26,561 chunks in all) |
| Characters sent | 24.8 million |
| Time waiting on oMLX | 368 s |
| Throughput | **72 chunks/s**, 67,000 characters/s |
| Index file ÷ raw f32 vectors | 0.536 (58.3 MB) |

At this rate the 30k text fixture (about 0.6–0.9 million chunks) would
take about 2.5–3.5 hours of oMLX time at the Run duty cycle's full speed.
At duty 0.5 that is roughly twice as long: still within "overnight"
(phase2-spec §13). Slice 2.4 measures tokens per second and 1,024 against
512 dimensions properly.

## Meaning search (Slice 2.4)

**P2-17: the vague question set against oMLX.** The product owner ran it
on the reference machine on 2026-11-09 with `just eval-semantic`.

- **Model and corpus:** Qwen3-Embedding-0.6B-8bit, with its query
  instruction. The spike corpus's 19 readable documents make 1,954 chunks.
- **The 512 column** is Atlas's own index (it stores 512). The 1,024 column
  is the same chunks embedded again at full length for comparison.
- **The floor:** a chunk counts as a meaning match from cosine similarity
  0.25.

| Hit@5 of 40 | Text only | Meaning only | Hybrid |
| --- | ---: | ---: | ---: |
| 1,024 dimensions | 0.875 | 0.950 | 0.925 |
| **512 dimensions (stored)** | 0.875 | **0.975** | **0.950** |

- **The gate:** hybrid hit@5 ≥ 0.80 and meaning only above text only both
  hold. Phase 1's set through hybrid search on the stored index is 40/40.
- **Query embedding** takes 16 to 17 ms median and 34 ms at most on oMLX.
- **Similarity:** relevant documents' best chunks average 0.50 (lowest
  0.26), and other documents' 0.44. The floor only keeps out nonsense; it
  can't separate relevant from related.
- **The dimension default (§6.1):** 512 loses nothing against 1,024, so
  Qwen3-Embedding is stored at 512 (`storedDimensions` in the bundled
  catalog). Vectors and the index file are half the size.
- **The set is less vague than intended:** text search alone finds 35 of
  the 40. Meaning lifts the hard ones, for example:
  - v06 ("verses about a grief that refuses to end"): text 9th, hybrid 3rd;
  - v11: text 10th, hybrid 4th;
  - v14: text 6th, hybrid 2nd;
  - v38 ("a jetliner's height-above-ground instrument"): text misses it,
    meaning finds it first. Hybrid still misses it, because partial text
    matches that also match by meaning outrank a meaning-only result.
- **With the fake backend on every OS** (`tests/semantic_eval.rs`):
  Phase 1's set is 39/40 through hybrid search, and every question gets a
  page with meaning search ready.

**Search at 200 results** (`benches/vectors.rs`, a million 1,024-dimension
vectors on the reference machine): hybrid search asks for the 200 nearest
chunks.

| Search | Recall@10 | p95 at 10 results | p95 at 200 results |
| --- | ---: | ---: | ---: |
| Unfiltered | 0.949 | 0.78 ms | 0.92 ms |
| One folder (5%) | 0.989 | 6.9 ms | 10.3 ms |
| One folder (0.5%) | 0.990 | 60 ms | 85 ms |

Even the narrowest folder stays within the 250 ms search budget, so there
is no separate exact-search path for small scopes.

**P2-18: hybrid search while the embedding job runs** (`benches/embedding.rs`):

- **The job:** the embed job with the deterministic backend at 1,024
  dimensions, at its Run duty cycle.
- **The queries:** every 200 ms, 10 queries (the text search mix plus
  vague questions) go through hybrid search uncached, each waiting 50 ms
  for its query embedding, as a local service would.

| Fixture | Machine | Documents | Chunks | Chunks/s | Bytes per vector | File ÷ raw | Save | Hybrid p50 / p95 / max |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 1k text | M5 Pro | 1,001 | 24,606 | 408 | 2,528 | 0.536 | 98 ms | 60 / 85 / 101 ms |
| 30k text | M5 Pro | 30,001 | 733,668 | 178 | 2,597 | 0.536 | 1,864 ms | 145 / **228** / 813 ms |

**P2-18 holds:** hybrid search p95 is 228 ms while the embedding job fills
the 30k text fixture, under the 250 ms target.

- **The run:** the job embedded 733,668 chunks in 69 minutes, slower as
  the index grew and with searches every 200 ms. The catalog grew 60.6 MiB
  per 1,000 documents, and the index file reached 1.5 GB (1,024 dimensions;
  Qwen3-Embedding is now stored at 512, half that).
- **The slowest search (813 ms)** coincided with the job holding the index
  for its adds. Searches wait for adds and removes, not for saves
  (ADR 0013).
- **The fake answers at once,** so the job's pace here is Atlas's own work.
  With oMLX the service sets it (about 72 chunks/s, Slice 2.3).

**Throughput and recall on real vectors** (`just eval-embed`, run by the
product owner on 2026-11-10, release build):

- **The setup:** oMLX with `Qwen3-Embedding-0.6B-8bit`. Every chunk of the
  spike corpus (1,954) was embedded again in the job's batches (126
  requests of at most 16 chunks and 16,000 characters), timed at the
  service.
- **The recall queries:** the 40 vague questions and 200 chunks, the
  index's top 10 against an exact cosine scan.

| Measure | 1,024 dimensions (the service's) | 512 (stored) |
| --- | ---: | ---: |
| Chunks per second | 48.4 | the same |
| Tokens per second (counted by the service) | 12,134 | the same |
| Characters per second | 47,011 | the same |
| Recall@10 of the index on real vectors | **0.9933** | **0.9979** |

- **512 costs the service nothing less:** it always computes 1,024
  numbers, and Atlas cuts them. The gain is storage and search, not
  embedding time.
- **Recall clears the ≥ 0.95 target** on real vectors at both lengths.
  Synthetic vectors gave 0.960 at a million (Slice 2.2).
- **The pace differs from Slice 2.3's 72 chunks/s** (67,000 characters/s).
  Chunk size doesn't explain it: both average about 950 characters. The
  two runs differ in library (1,000 generated books there), build (debug
  there) and who sends the requests (the job there, this test here). At 48
  to 72 chunks/s the 30k text fixture (0.6 to 0.9 million chunks) would
  take 2.5 to 5 hours: overnight, but not measured.
- **The share only meaning finds:** of the 40 vague questions, text alone
  puts a relevant document in the top 5 for 35, and hybrid search at the
  stored 512 for 38. Three (8%) are found only with meaning: v06 ("verses
  about a grief that refuses to end"), v11 and v14.

## Related documents (Slice 2.5)

`benches/embedding.rs`, after the job has finished: related documents for
200 documents spread over the library, each asked twice, first cold and
then from the cache. Ten related documents are asked for. The fake
backend's vectors are spread more evenly than a real model's, so every
document finds ten.

| Fixture | Machine | Documents | Related p50 / p95 / max (cold) | p95 (cached) | Related per document |
| --- | --- | ---: | --- | ---: | ---: |
| 1k text | M5 Pro | 1,001 | 1.8 / 2.0 / 2.5 ms | 1.9 ms | 10 |
| 30k text | M5 Pro | 30,001 | 2.8 / 3.1 / 3.4 ms | 2.9 ms | 10 |

- **Well under the spec's estimate** of about 5 ms at 1,000 documents,
  and still about 3 ms at 30,000 (733,668 vectors).
- **The cache saves little.** Averaging 64 vectors and one index search
  are cheap. Most of the time goes to looking up each related document's
  file, which is done afresh every time so a missing file drops out at
  once.

## Ask Atlas (Slice 2.6)

`just eval-ask`, run by the product owner on the reference machine: the
spike corpus embedded through a real embedding model, as for
`eval-semantic`. Then each of the 40 vague questions is asked through a
real chat model, and from Slice 2.6.2 also 12 questions the corpus has
nothing on (`questions-off.json`). It reports, for each set:

- the time to the first word and to the whole answer;
- the sources sent, cited and invented;
- whether a document relevant to the question was among the sources.

It asserts only that no invented citation reaches the answer (spec §13's
citation and dropped-citation rates).

<!-- ask-eval -->
**The owner's run** (2026-11-10, release build, the reference machine):

- **Services:** oMLX for both. Embedding through `Qwen3-Embedding-0.6B-8bit`
  (the corpus took 35.1 s). Answers through `Qwen3.8-27B-MLX-4bit`, a
  thinking model, with Atlas's defaults: at most 4,096 tokens, temperature
  0.2, thinking hidden.
- **The run:** 40 questions, one after another, 5,572 s in all.

| Measure | Result |
| --- | ---: |
| First word, p50 / p95 | 88.5 s / 226.9 s |
| Whole answer, p50 / p95 | 118.9 s / 270.3 s |
| Questions with a relevant document among the sources | 39 / 40 |
| Sources per question | 12.9 on average (5 to 15) |
| Citation rate (cited ÷ supplied) | 0.20 (101 / 514) |
| Invented citations | **0** |
| Flagged as weak retrieval | 0 |
| Answers with no text at all | **5** (v04, v13, v19, v29, v35) |
| Answers saying the excerpts don't cover the question | 8 |

What the run shows:

- **Retrieval is good; citations are honest.** A relevant document was
  among the sources for 39 questions. The model never cited a number it
  wasn't given, so the parser had nothing to remove.
- **The time is mostly thinking.** The first word waits until the model
  stops reasoning. A source-heavy prompt of about 6,000 tokens plus up to
  4,096 tokens of thinking, at about 15 tokens/s, gives the 1.5 to 4.5
  minutes measured. That is too slow for the screen.
- **Five answers came back empty.** Each took 264 to 271 s, about 4,096
  tokens at that rate: the model spent the whole allowance thinking and
  never wrote an answer. The engine reported these as `done` with no text,
  which the screen must not show as an answer. (v34 nearly did the same:
  246 s thinking, 15 s writing.)
- **"Not covered" answers cite everything.** When the model says the
  excerpts don't answer, it sometimes cites every source (v09 11 of 11,
  v12 14 of 14, v15 11 of 13). That inflates the citation rate and is
  wrong on screen.
- **The right document, the wrong passage.** Eight answers said the
  excerpts don't cover the question although the relevant document was
  there. For example, Candide was sent, but not its earthquake. For the
  道德經 the excerpt sent was the Project Gutenberg licence text.
- **The weak-retrieval flag never fired.** With 8 documents giving 10 to 15
  sources, "fewer than 3" can't happen. Every question here also has some
  document with every word, or a meaning match above the margin. As
  defined, the flag doesn't tell a covered question from an uncovered one.
- **Formatting.** Six answers used Markdown emphasis (`*Title*`,
  `**FOXDEN**`) despite "plain text".
- **Words run together** (`sorrowover`, `haslittle`) in the pasted
  output were not in the answers. The second run's whole answers have
  every space. The joins sat where the terminal wrapped its lines, and
  copying the wrapped text lost the space.

**Slice 2.6.1** answers these findings ([ADR 0015](adr/0015-ask-atlas-answers-citations-injection.md)):

- thinking is asked to be off;
- an answer with no text is the error `no_answer`;
- a "not covered" answer cites nothing;
- Project Gutenberg's header and licence are cut from excerpts;
- weak retrieval goes by the meaning margin alone, which `just eval-ask` now
  prints for each question.

**The run after Slice 2.6.1** (2026-11-10, the same machine, services and
model, thinking off; 976 s in all):

| Measure | Thinking on | **Thinking off** |
| --- | ---: | ---: |
| First word, p50 / p95 | 88.5 s / 226.9 s | **14.4 s / 17.9 s** |
| Whole answer, p50 / p95 | 118.9 s / 270.3 s | **23.4 s / 31.2 s** |
| A relevant document among the sources | 39 / 40 | 39 / 40 |
| Sources per question | 12.9 | 12.1 |
| Citation rate (cited ÷ supplied) | 0.20 | 0.20 (95 / 484) |
| Invented citations (removed) | 0 | 6, all in v23 |
| Empty answers | 5 | **0** |
| Answers saying the excerpts don't cover the question | 8 | **18** |
| Flagged as weak retrieval | 0 | 0 |
| Margin above the floor, min / median / max | — | 0.22 / 0.39 / 0.51 |

What it shows:

- **Fast enough.** The first word now waits mostly on reading the
  excerpts, about 6,000 tokens. The answer itself takes about 9 s at
  about 17 tokens/s. The model can stay.
- **Too literal without thinking.** 18 answers begin "The excerpts do not
  contain…", although the relevant document was among the sources in 17
  of them. The vague set's questions describe a work rather than ask a
  fact. Without reasoning, the model won't say "that is The Raven" unless
  an excerpt says so in the question's words:
  - v04 (the visitor repeating one word) was sent The Raven, but before
    "Nevermore".
  - v13 (the tutor) was sent an excerpt naming Pangloss, and still
    refused.
- **The citation rule half took.** No answer cites every source to say
  they don't apply, except v12 (13 of 14). But no "not covered" answer
  cites nothing either: each goes on to say what is close, as rule 4
  allows, and cites that. So "answers citing nothing" (0) doesn't
  measure "not covered"; the phrase count above does.
- **The parser did its job.** v23's six invented numbers were removed
  before they reached the text.
- **The margin can't be tuned from this set.** Every question has a
  document in the corpus, and their margins (0.22 to 0.51) overlap
  whether the answer was found or not. The weak flag needs questions the
  library has nothing on.

**Slice 2.6.2** answers these findings
([ADR 0015](adr/0015-ask-atlas-answers-citations-injection.md)):

- a question describing a work is answered by naming the source that
  matches;
- each document can give a second passage by meaning, within the same
  budget, with excerpts of at most 2,000 characters;
- the 12 off-library questions tune the weak margin.

**The run after Slice 2.6.2** (2026-11-10, the same machine, services and
model; 1,341 s for 52 questions):

| Measure | Vague set (40) | Off set (12) |
| --- | ---: | ---: |
| First word, p50 / p95 | 15.3 s / 18.8 s | 16.3 s / 17.8 s |
| Whole answer, p50 / p95 | 25.6 s / 37.2 s | 22.1 s / 24.2 s |
| A relevant document among the sources | 39 / 40 | — |
| Sources per question | 14.2 | 14.0 |
| Citation rate (cited ÷ supplied) | 0.19 | **0.68** |
| Invented citations (removed) | 2 | 0 |
| Answers saying the excerpts don't cover it | **4** (from 18) | **12 of 12** |
| … citing nothing | 0 | 3 |
| Flagged as weak retrieval (margin below 0.10) | 0 | 5 |
| Margin above the floor, min / median / max | 0.22 / 0.39 / 0.51 | 0.04 / 0.12 / 0.24 |

What it shows:

- **The described works are named.** v04 now quotes "Nevermore" from a
  second Raven passage, and v13 names Pangloss. v06 and v11 name The Raven
  and Candide. Of the 4 "not covered" answers:
  - v14 is a good answer that the eval's phrase test misread ("ruling by
    doing nothing");
  - v23 and v28 are honest partial answers;
  - v38's document wasn't retrieved, the one miss in both runs.
- **Off-library questions are refused, and nothing is made up.** All 12
  say the library doesn't cover them. The three famous works the corpus
  lacks (o05, o10, o12) are not claimed.
- **But refusals inventory the sources.** 9 of the 12 go on to list what
  every excerpt covers, citing each ("The sources cover Alice [1][3][4]…"),
  hence the 0.68 citation rate. Rule 5 ("cite nothing") isn't enough on its
  own.
- **The margin separates the sets.** The off set reaches 0.24 at most, and
  the vague set starts at 0.22. A weak margin of 0.20 would flag no vague
  question and at least half the off set; 0.10 flags 5 of 12. These
  margins are for Qwen3-Embedding-0.6B with its 0.25 floor.
- **Second places cost little.** About 2 more sources per question; the
  first word came 1 s later.

**Slice 2.6.3** sets the weak margin to 0.20. A refusal is one sentence
that stops: it doesn't list what the excerpts cover, and cites nothing.
The eval no longer reads "nothing" as a refusal, and the answers file
gives each question's margin.

**The run after Slice 2.6.3** (2026-11-10, the same machine, services and
model; 1,316 s for 52 questions):

| Measure | Vague set (40) | Off set (12) |
| --- | ---: | ---: |
| First word, p50 / p95 | 15.6 s / 19.1 s | 15.6 s / 17.1 s |
| Whole answer, p50 / p95 | 25.7 s / 33.9 s | 19.2 s / 21.8 s |
| A relevant document among the sources | 39 / 40 | — |
| Citation rate (cited ÷ supplied) | 0.20 | **0.18** (from 0.68) |
| Invented citations (removed) | 3 | 0 |
| Answers saying the excerpts don't cover it | 3 | 12 of 12 |
| … citing nothing | 0 | **9** (from 3) |
| Flagged as weak retrieval (margin below 0.20) | **0** | **8** (from 5) |

What it shows:

- **The vague set is as good as this model and corpus allow.** The 3 "not
  covered" answers are v23 and v28, which say honestly that no excerpt
  settles the question, and v38, whose document wasn't retrieved.
- **Refusals are short now.** 9 of 12 are one uncited sentence. Three
  still list the excerpts' subjects with citations (o02, o08, o11).
- **The weak flag works.** No vague question is flagged. The 4 off-library
  questions not flagged (o02 0.21, o08 0.22, o12 0.22, o10 0.24) are
  inside the vague set's range, which starts at 0.22. No margin separates
  them, and each was still refused.
- **Ask Atlas is ready for its screen** with this model: about 16 s to the
  first word and 26 s to the whole answer, citations checked, nothing
  invented reaching the text.

CI has no model. There, the engine runs against a scripted service
streaming canned answers (`tests/answers.rs`, P2-13 to P2-16).

<!-- results:end -->
