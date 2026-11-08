# Phase 0 close-out review

- **Date:** 2026-11-08
- **Against:** [Phase 0 spec](phase0-spec.md) v0.1, revision 2: §1
  (definition of done), §12 (required tests), §13 (benchmarks), §14
  (packaging and CI), §16 (acceptance checklist)
- **State reviewed:** `main` after Slice 0.9 (tag `phase0/slice-09`), plus
  the fixes listed under "Fixed during this review"
- **Sign-off:** the §16 checklist is the product owner's. This review gives
  the evidence and a recommendation; it doesn't tick the boxes.

## Verdict

Phase 0 meets its definition of done, with documented gaps on Windows
and Linux. The spec allows this (§1: "If an OS is unavailable, document it
as unverified, with a reproducible checklist").

- **macOS arm64.** Every item was verified end to end on real hardware,
  including the release DMG with the native folder dialogs, Preview and
  Finder, an exFAT drive unplug, and real battery readings.
- **Windows x64 and Linux x64.** Every item was verified in CI: all
  required automated tests, release installers, benchmarks, and an
  automated click-through of the built app. That click-through uses
  debug-only stand-ins for the folder dialog and for opening files. So the
  native dialogs, the real OS open and reveal, the OS keychains, physical
  drive and power changes, and installing on a real desktop were not
  exercised. They are listed below, with the manual checklist that covers
  them ([platform-validation.md](platform-validation.md#manual-checklist-for-windows-and-linux)).

The product owner's decision in Slice 0.8 was "CI only, document gaps"
for Windows and Linux. If you want full sign-off without gaps, the manual
checklist takes roughly 45 minutes per OS on a real machine.

## Definition of done (§1)

✅ verified · ◐ verified with documented gaps · ❌ not verified

| # | Item | macOS arm64 | Windows x64 | Linux x64 | Evidence |
| --- | --- | :-: | :-: | :-: | --- |
| 1 | First-run onboarding | ✅ | ✅ | ✅ | `App.test.ts` (first run opens on Welcome, no sign-in or AI gate); smoke step "launch shows the welcome screen" on Windows and Linux; macOS release smoke |
| 2 | Data root and folders persist across restarts | ✅ | ◐ | ◐ | P0-02; smoke "quit and relaunch reopens the same library". Windows and Linux pick folders through the test hook, not the native dialog |
| 3 | Device check gives nonblocking guidance | ✅ | ◐ | ◐ | Slice 0.4 (macOS real readings + simulated states); Windows and Linux probes run in CI tests; their real battery hardware is unverified |
| 4 | Throttled inventory into SQLite inside the data root, Library updates progressively | ✅ | ✅ | ✅ | P0-05, P0-15; benchmarks on all three OSes; smoke "add a library folder and see rows" |
| 5 | Open in the OS default application | ✅ | ◐ | ◐ | macOS: Preview and Finder by hand (0.6, 0.8). Windows and Linux: P0-16 and the smoke test check the exact path handed to the opener; the OS action itself is unverified |
| 6 | Pause/resume; forced kill resumes without corruption or duplicates | ✅ | ✅ | ✅ | P0-08 (real process kill), P0-09; benchmark recovery (0 duplicate rows) and smoke pause/resume on all three |
| 7 | Battery, low space, source disconnect handled and explained | ✅ | ◐ | ◐ | P0-10–P0-14 on all three (simulated readings, injected sources). macOS: real exFAT unplug of the data drive and of a source drive, real on-battery state. Physical events on Windows and Linux are unverified |
| 8 | Test a local model service honestly; AI not needed to browse | ✅ | ◐ | ◐ | P0-17–P0-19 on all three; macOS against real Ollama and oMLX. Credential Manager (Windows) and Secret Service (Linux) storing a real key are unverified |
| 9 | Test, smoke and packaging evidence for all three OSes | ✅ | ✅ | ✅ | [platform-validation.md](platform-validation.md), [packaging.md](packaging.md), [benchmarks.md](benchmarks.md); CI on every push |
| 10 | Extraction and vector-engine ADR with numbers, licenses, sizes, build times | ✅ | ✅ | ✅ | [ADR 0008](adr/0008-extraction-and-vector-engines.md), [spike results](spike-0.9.md); Spikes run [42505069661](https://github.com/markcheno/Atlas/actions/runs/42505069661) |

## Required automated tests (§12)

All 24 exist and run in CI on every push, on all three OSes unless noted.

| ID | Test(s) | Notes |
| --- | --- | --- |
| P0-01 | `App.test.ts` "first run opens on Welcome…"; smoke step 1 | |
| P0-02 | `workspace::p0_02_create_then_reopen_after_restart` | |
| P0-03 | `workspace::p0_03_newer_schema_is_refused_without_writes` | |
| P0-04 | `workspace::p0_04_second_process_is_blocked_by_the_lock` | real second process |
| P0-05 | `inventory::p0_05_catalogs_a_nested_tree_without_touching_it`, `sources::p0_05_sources_are_never_modified` | snapshots of names, sizes, times |
| P0-06 | `sources::p0_06_*` (2), `inventory::p0_06_overlapping_roots_are_not_double_scanned` | |
| P0-07 | `inventory::p0_07_symlinks_are_skipped_and_counted` | macOS and Linux only: Windows symlinks need extra privileges; Windows junctions aren't tested |
| P0-08 | `inventory_crash::p0_08_kill_mid_scan_then_resume` | real SIGKILL / TerminateProcess |
| P0-09 | `inventory::p0_09_user_pause_persists_across_restart` | |
| P0-10 | `policy::p0_10_battery_hysteresis`, `inventory::p0_10_low_battery_pauses_until_recovered` | 19/20/21 sequence included |
| P0-11 | `policy::p0_11_*`, `inventory::p0_11_unknown_power_runs_throttled` | |
| P0-12 | `policy::p0_12_*`, `inventory::p0_12_low_space_pauses_without_writing` | |
| P0-13 | `inventory::p0_13_source_disappearing_mid_scan_marks_nothing_missing`, `sources::p0_13_unplugged_folders_are_kept` | |
| P0-14 | `workspace::p0_14_missing_data_root_is_not_recreated` | |
| P0-15 | `library::p0_15_results_grow_while_scanning` | |
| P0-16 | `open::p0_16_hostile_names_are_passed_through_exactly`, `open::p0_16_missing_file_is_reported_without_opening` | per-OS hostile name sets |
| P0-17 | `providers::p0_17_ollama_lists_models_without_sending_anything` | scripted local HTTP server |
| P0-18 | `providers::p0_18_api_key_never_leaves_the_keychain` | |
| P0-19 | `providers::p0_19_plain_http_rules` | |
| P0-20 | `sources::p0_20_data_root_rules`, `inventory::p0_20_data_root_inside_the_source_is_skipped` | |
| P0-21 | `inventory::p0_21_closing_stops_the_worker_and_reopening_resumes` | |
| P0-22 | `inventory::p0_22_unreadable_folder_is_bounded_and_protects_only_its_subtree`, plus invalid-path tests in `sources` and `workspace` | paths over 260 characters on Windows aren't tested |
| P0-23 | `workspace::p0_23_network_and_cloud_volumes_are_refused` | |
| P0-24 | Linux: `sources::p0_24_non_utf8_folder_names`, `library::p0_24_non_utf8_names_are_listed`; Windows: `sources::p0_24_unpaired_surrogate_folder_names`, `library::p0_24_unpaired_surrogate_names_are_listed`; both: `open::p0_24_odd_names_open_from_native_bytes` | macOS file systems refuse such names. The Windows tests and the open test were added in this review |

The spec also asks for migration tests (`migrations` unit tests) and for
checking that source files' hashes and metadata don't change (P0-05).
Both exist.

## Benchmarks (§13)

| Requirement | Status |
| --- | --- |
| Launch to usable window; folder selection to first rows | ✅ M5 Pro release 1.5 s; CI smoke 1.0 s (Linux), 2.6–2.9 s (Windows); first rows 25–98 ms (catalog), 0.4–0.6 s through the UI |
| Throughput, CPU, peak RAM, disk growth, responsiveness on 1k and 30k | ✅ [benchmarks.md](benchmarks.md), four machines |
| AC vs battery; pause/resume; concurrent browsing | ✅ simulated power on all machines; real battery on the M5 Pro |
| Recovery after forced termination | ✅ 0 duplicate rows on every OS |
| DB/WAL size; cold and warm query latency | ✅ |
| macOS device first; Windows and Linux CI evidence | ✅ |
| Provisional p95 < 250 ms on 1k while scanning | ✅ 2.0–3.5 ms |
| Throttle revisited from measurements | ✅ 100 → 500 entries/s ([ADR 0004](adr/0004-resource-policy.md) amendment) |

## Packaging, CI and release evidence (§14)

| Requirement | Status |
| --- | --- |
| CI matrix with Rust and TypeScript checks, package builds, artifact upload | ✅ `ci.yml`, every push |
| `.app`/`.dmg`, NSIS `.exe`, AppImage and `.deb` | ✅ debug and release, every push to `main` |
| Expected OS security prompts documented; not called production-ready | ✅ [packaging.md](packaging.md) |
| Smoke on all three OSes (launch, choose directories, scan, open, pause/resume, restart) | ◐ macOS by hand; Windows and Linux automated, with test hooks for the dialog and the opener |
| Linux dependencies documented, no universal-compatibility claim | ✅ Ubuntu 24.04 only |
| No Python, Docker, SQLite server, Ollama or LLM needed to start | ✅ (and ADR 0008 keeps Python out of the installer) |
| Lockfiles and a license report; GPL/AGPL and bundled-binary obligations flagged | ✅ [dependency-licenses.md](dependency-licenses.md), [spike-dependency-licenses.md](spike-dependency-licenses.md) |

## Acceptance checklist (§16): assessment

| Item | Assessment | Evidence |
| --- | --- | --- |
| Starts without terminal setup, Docker, Python, account, or LLM | ✅ | Installers on all three OSes; first run needs no account or AI |
| Choose a data directory; reopen the same catalog after restart | ✅ | P0-02, smoke relaunch, macOS by hand |
| Add multiple roots; no source files altered or moved | ✅ | P0-05 (file snapshots before and after), P0-06 |
| Browse progressively discovered PDFs and EPUBs with filename search | ✅ | P0-15, smoke search "CAFE creme" |
| Open an original in the OS default application | ✅ macOS · ◐ Windows, Linux | See §1 item 5 |
| Pause, restart, and resume after a forced kill without duplication | ✅ | P0-08, P0-09, benchmark recovery |
| Understandable resource, battery and low-space guidance; never monopolizes the machine | ✅ | P0-10–P0-12; about 7% of one core at the default rate |
| Persistent metadata and indexes beneath the data root, exceptions documented | ✅ | Exceptions in [ADR 0001](adr/0001-local-ipc-no-server-single-data-root.md): keychain secrets, the locator file, OS-managed caches, separately installed runtimes |
| Test a local Ollama endpoint without uploading documents; secure credentials | ✅ macOS · ◐ Windows, Linux keychains | P0-17–P0-19; real Ollama on macOS |
| No unprotected HTTP server, SQL, shell or filesystem access from the frontend | ✅ | Capabilities: `core:default` only; no HTTP server ([ADR 0001](adr/0001-local-ipc-no-server-single-data-root.md)); the opener runs no shell (P0-16) |
| Recorded test, build, license and platform-smoke evidence | ✅ | This document and the files it links |
| Distinguishes what works now from future AI and cloud features | ✅ after this review's fix | README capability table; AI Models only tests connections. A Settings sentence still said cataloging was "coming in slice 0.5"; fixed |
| Extraction and vector-engine ADR with measured numbers, licenses, binary-size costs | ✅ | [ADR 0008](adr/0008-extraction-and-vector-engines.md) |

## Fixed during this review

- **Stale UI copy.** Settings › Performance & Battery said "When Atlas
  catalogs your folders (coming in slice 0.5)…". Cataloging has worked
  since 0.5. The text now reads "When Atlas catalogs your folders…", and
  a test asserts that the sentence never mentions a slice or "coming".
- **Dead stubs that claimed to be unimplemented.** `CatalogRepository`
  still declared `list_documents` and `file_instance`. Their only
  implementation returned "not implemented, planned for 0.6", although
  browsing and opening were built in 0.6 on the read-only pool. Nothing
  called them. They are removed, and the trait's documentation and the
  architecture table now point at the real implementations.
- **P0-24 had only half its coverage, and the missing half hid a bug.**
  It ran on Linux only, and never opened a file. This review added the
  Windows half (unpaired-surrogate names) and an open-from-native-bytes
  test for both. On Windows the new test failed: such a file was refused
  with "outside its folder".

  Slice 0.8 switched to `dunce::canonicalize` to show ordinary `C:\…`
  paths. dunce keeps the verbatim `\\?\C:\…` form for names that aren't
  valid Unicode, while their folder comes back plain, so the textual
  containment check never matched.

  Path comparisons now go through `native_path::comparable`, which rewrites
  verbatim prefixes for comparing only. That covers the containment check
  when opening, the path-based fallback for folder overlap, and the
  same-library check. Folder overlap normally compares file identities and
  wasn't affected. CI run: @@CLOSEOUT_CI@@.
- **Capability description.** It called itself "Slice 0.1 baseline"; the
  wording now describes the whole app.

## Not verified (carried forward)

Each item is covered by the
[manual checklist for Windows and Linux](platform-validation.md#manual-checklist-for-windows-and-linux)
or is planned work.

| Gap | Platforms | Covered by |
| --- | --- | --- |
| Native folder dialogs | Windows, Linux | Manual checklist (Data location, Library folders) |
| Opening and revealing in the OS | Windows, Linux | Manual checklist (Library, search, open) |
| Credential Manager / Secret Service with a real key | Windows, Linux | Manual checklist (AI services) |
| Removable-drive unplug; real battery and AC changes | Windows, Linux; AC unplug and replug also on macOS | Manual checklist (Device check, Cataloging) |
| Real low disk space (only simulated) | all | Tests P0-12; manual checklist |
| Installer experience: SmartScreen, Gatekeeper dialog, uninstall | all | Manual checklist (Installers) |
| Symlinks and junctions on Windows (P0-07) | Windows | Phase 1 |
| Paths over 260 characters | Windows | Phase 1 |
| Other Linux distributions, Wayland, ARM | Linux | Not claimed ([packaging.md](packaging.md)) |
| Placeholder app icons | all | Before any public build |

## Carry into Phase 1

These come from Phase 0's measurements and decisions, not from new
requirements:

- **Extraction:** PDFium (pdfium-render) in a helper process; rbook +
  html2text for EPUB ([ADR 0008](adr/0008-extraction-and-vector-engines.md)).
  The helper process brings with it:
  - a garbled-text check that routes documents to OCR;
  - EPUB entry-size caps and DRM detection;
  - PDFium shipped as a shared library with its notices, and included in
    signing.
- **Jobs:** extraction is the second job kind, which triggers moving
  `scan_sessions` into a general jobs table (phase0-spec §6).
- **Full-text search:** FTS5 in the catalog, benchmarked on the 30k set
  with realistic text volume (phase0-spec §11, product spec).
- **Resuming inside large folders:** a pause in the middle of a
  20,000-entry folder takes about 18 s to resume at the policy rate
  ([benchmarks.md](benchmarks.md)). Worth fixing before extraction makes
  scans longer.
- **Before Phase 1 starts:** set the target share of the evaluation
  questions that full-text search must answer (the product spec's Phase 1
  done criterion asks for it).
