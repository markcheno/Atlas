# Phase 1 close-out review

- **Date:** 2026-11-09
- **Against:** [Phase 1 spec](phase1-spec.md) v0.1 and its revision notes
  (through Slice 1.8): §1 (definition of done), §12 (required tests), §13
  (benchmarks), §14 (packaging, CI and release evidence), §16 (acceptance
  checklist)
- **State reviewed:** `main` after Slice 1.8 (tag `phase1/slice-08`,
  version 0.2.0), including the fixes listed under "Fixed during this
  review"
- **Sign-off:** the §16 checklist is the product owner's. This review gives
  the evidence and a recommendation; it doesn't tick the boxes.

## Verdict

Phase 1 meets its definition of done, with documented gaps on Windows and
Linux, as the spec allows (§1: "If an OS is unavailable, document it as
unverified with a reproducible checklist").

- **macOS arm64.** Verified end to end on real hardware: the CI-built
  release DMG with PDFium inside, run by the product owner with the native
  dialogs, Preview and Finder (Slice 1.8); the 30k text fixture read to the
  last file on the reference machine and on the macOS runner; every
  required test; the benchmarks against the §13 targets.
- **Windows x64 and Linux x64.** Verified in CI: every required test
  (including P1-21's junctions on Windows), release installers whose worker
  says hello, the benchmarks on the 1k fixtures, the 30k text fixture read
  to the last file (the evidence run), and an automated click-through of
  the built app that reads text, searches a passage, opens a result, pauses
  and resumes reading, and quits with no worker left. As in Phase 0, that
  click-through uses debug-only stand-ins for the folder dialog and for
  opening files, so the native dialogs, the real OS open and reveal, the OS
  keychains and physical power and drive changes are not exercised; they
  are listed below with the checklist that covers them
  ([platform-validation.md](platform-validation.md#manual-checklist-for-windows-and-linux)).

The product owner's Phase 0 decision ("CI only, document gaps") stands for
Windows and Linux.

## Definition of done (§1)

✅ verified · ◐ verified with stand-ins or simulated inputs · ❌ not verified

| # | Item | macOS arm64 | Windows x64 | Linux x64 | Evidence |
| --- | --- | :-: | :-: | :-: | --- |
| 1 | Background reading with a per-file status and a reason | ✅ | ✅ | ✅ | P1-04 (every corpus document in its expected status, each with a reason), P1-09, P1-10, P1-13; smoke "search finds a passage" waits for "Text up to date"; release DMG smoke step 4 |
| 2 | Evaluation corpus: relevant document in the top 5 for ≥ 80% | ✅ | ✅ | ✅ | P1-17 on every OS: 40 of 40 questions, every one at rank 1 ([ADR 0010](adr/0010-full-text-search.md)). The corpus has only 17 searchable documents, so this is a floor check |
| 3 | 30k text fixture: 100% of well-formed files read; every other file reported | ✅ | ✅ | ✅ | Evidence run [42685716386](https://github.com/markcheno/Atlas/actions/runs/42685716386): 32,006 of 32,006 files "Text ready" on every runner (445 s on macOS, 1,145 s on Windows, 431 s on Linux, unthrottled), none waiting; M5 Pro the same. Every other file reported: P1-04, P1-05 |
| 4 | Rescans read only changed files; originals unchanged | ✅ | ✅ | ✅ | P1-03 (one changed file re-read; snapshot of bytes, times and hashes unchanged); every extraction test compares the originals before and after |
| 5 | Slows on battery, pauses at ≤ 20%, low space, missing roots and quit; resumes without redoing; no partial document | ✅ | ◐ | ◐ | P1-14 (simulated 50% → 20% → 25%), P1-12 and P1-22 (real kills), P1-02; smoke "pause and resume reading" and "quit while reading leaves no worker process" on Windows and Linux; macOS by hand (Slice 1.7) and the DMG smoke. Real battery hardware on Windows and Linux is unverified |
| 6 | Search p95 under about 250 ms on the 30k text fixture while reading | ✅ | ◐ | ◐ | P1-19 on the reference machine: p95 158 ms while reading the 30k text fixture ([benchmarks](benchmarks.md#phase-1-summary-slice-18)). Windows and Linux report the 1k fixture only (the 30k search run took over 40 minutes on a Windows runner; revision note, Slice 1.4) |
| 7 | Identical content is one document with all locations; moves keep their text | ✅ | ✅ | ✅ | P1-06, P1-07; Library › Duplicates (`tests/duplicates.rs`, smoke "duplicates list the six Introduction copies", DMG smoke step 7) |
| 8 | Tests, smoke, benchmarks, packaging with PDFium and license evidence on three OSes | ✅ | ✅ | ✅ | [platform-validation.md](platform-validation.md), [benchmarks.md](benchmarks.md), [packaging.md](packaging.md), [dependency-licenses.md](dependency-licenses.md); CI on every push |

## Required automated tests (§12)

All run in CI on macOS arm64, Windows x64 and Linux x64 unless noted.
Tests are in `src-tauri/tests/`.

| ID | Test(s) | Notes |
| --- | --- | --- |
| P1-01 | `jobs::p1_01_upgrade_keeps_a_paused_scan_and_its_frontier` | a schema-6 catalog |
| P1-02 | `jobs::p1_02_a_scan_and_an_extract_job_run_together_and_pause_independently`, `jobs::p1_02_job_commands_check_state_for_every_kind` | |
| P1-03 | `extraction::p1_03_only_changed_files_are_read_again` | snapshot of the originals |
| P1-04 | `extraction::p1_04_the_corpus_reaches_its_expected_statuses` | every manifest `expect`; each non-text status has a reason |
| P1-05 | `extraction::p1_05_p1_06_a_text_fixture_is_fully_read_and_copies_once` | 120 generated files in the test; the 30k gate is the evidence run (`bench.mjs --evidence`, `--require-all-read`) |
| P1-06 | the same test | one document, two instances, read once |
| P1-07 | `extraction::p1_07_a_moved_file_links_by_hash` | no re-read |
| P1-08 | inside P1-04 | no text document is taken for a scan; the scans and the no-ToUnicode PDF need OCR |
| P1-09 | `extraction::p1_09_drm_is_found_before_any_chapter_is_read`, and P1-04 | the owner-password PDF is read |
| P1-10 | `extraction::p1_10_oversized_unsafe_and_mislabelled_files` | |
| P1-11 | `extraction_crash::p1_11_crashing_and_hanging_files_do_not_stop_the_job`, plus `tests/supervisor.rs` | real worker with its debug fault hook |
| P1-12 | `extraction_crash::p1_12_kill_mid_extraction_then_resume` | real SIGKILL / TerminateProcess; FTS `integrity-check` |
| P1-13 | `extraction::p1_13_cloud_only_files_are_never_opened` | the worker gets no request |
| P1-14 | `extraction::p1_14_battery_slows_then_pauses_then_resumes` | simulated power |
| P1-15 | `search::p1_15_typed_operators_are_words_and_never_errors` | |
| P1-16 | `search::p1_16_phrases_accents_and_case` | |
| P1-17 | `search_eval::p1_17_evaluation_questions_hit_at_5` | prints each question's rank |
| P1-18 | `metadata::p1_18_titles_come_from_metadata_then_the_first_page_then_the_file_name`, `metadata::p1_18_papers_get_their_titles_from_the_first_page` | |
| P1-19 | `benches/search.rs` (`just bench-search`, `--max-p95-ms 250`) | asserted on the reference machine; reported on CI for the 1k fixture |
| P1-20 | `inventory::p1_20_resuming_inside_a_large_folder_charges_replayed_entries_less`, and the inventory benchmark's `--pause` | 0.85–0.99 s on four machines |
| P1-21 | `inventory::p1_21_junctions_are_skipped_and_counted` (Windows only), `extraction::p1_21_a_300_character_path_is_cataloged_and_read` (every OS) | |
| P1-22 | `extraction_crash::p1_22_a_hung_worker_does_not_outlive_a_killed_app`, the worker check in P1-12, smoke "quit while reading leaves no worker process" | |

The other §12 requirements: migration tests for 0007 to 0010 live with the
migrations (`catalog::migrations::tests::version_7_…` to `version_10_…`);
every crash test (P1-11, P1-12, P1-22, and the re-read crash test in
`tests/refresh.rs`) ends with SQLite's and FTS5's integrity checks; every
extraction test compares the originals before and after.

## Benchmarks (§13)

| Requirement | Status |
| --- | --- |
| Throughput, CPU at the Run duty cycle and unthrottled, worker RSS and restarts, app RSS | ✅ `benches/extraction.rs` on four machines ([benchmarks](benchmarks.md#text-extraction-slice-17)) |
| Catalog growth, WAL size, end-of-job merge | ✅ 1.62× on new catalogs (accepted, see below); WAL 16–31 MiB; merge under 1 ms |
| Time to the first searchable document; to a fully searchable 30k library | ✅ under 1 s on every machine; 30k read in 343 s at Run on the M5 Pro, 431–1,145 s unthrottled on the runners |
| Search latency while reading and idle, by query kind, cold and warm | ✅ [benchmarks](benchmarks.md#phase-1-summary-slice-18); P1-19 p95 158 ms |
| Pause and resume cost; recovery after a forced kill (0 duplicates) | ✅ P1-20; 0 duplicates, 0 partial documents and 0 workers left on every machine |
| AC versus simulated battery; the duty cycle's effect on wall time | ✅ Run and Slow on four machines; rests pooled to 100 ms after the benchmark showed millisecond rests cost more than they rested |
| macOS reference first; Windows and Linux CI evidence | ✅ |
| Provisional targets | Search p95 158 ms (≤ 250 ms) ✅; reading 777 pages/s at Run (≥ 200) ✅; growth 1.62× (≤ 1.5×) ◐ accepted by the product owner |

**Catalog growth (product-owner decision, Slice 1.8).** New catalogs
measure 1.62× the stored text on the 30k fixture (2.3× before Slice 1.7's
16 KiB pages). The remainder is the full-text index (0.40×), which exact
phrases need. Getting under 1.5× means compressing passages, which needs a
contentless index and snippets built by Atlas; it stays an option for a
later phase ([ADR 0010](adr/0010-full-text-search.md)). Catalogs created
before Slice 1.7 keep 4 KiB pages.

## Packaging, CI and release evidence (§14)

| Requirement | Status |
| --- | --- |
| PDFium pinned by hash, fetched and verified, bundled on three OSes with its notices | ✅ `tools.lock.json`, `scripts/fetch-pdfium.mjs`; every release bundle's worker says hello in CI |
| macOS signing gate: `codesign --verify --deep --strict`, worker hello from the `.app` | ✅ every full run; the library-validation entitlement is recorded for notarization review |
| CI: smoke gains "search finds a passage" and "quit leaves no worker process" | ✅ plus "pause and resume reading" and "open from a search result" (this review) |
| Bench job adds the 30k text scenario | ◐ CI runs the 1k text fixture (the 30k one took over 40 minutes on Windows runners; revision note, Slice 1.4); the 30k extraction gate runs as the opt-in evidence run, and 30k search on the reference machine |
| Licenses: the new crates and the PDFium native row; GPL/AGPL flagged | ✅ [dependency-licenses.md](dependency-licenses.md) regenerated in Slice 1.8 with no changes: 573 crates, 7 MPL-2.0 entries flagged for review, no GPL or AGPL |
| Linux: PDFium bundled, not a system package | ✅ [packaging.md](packaging.md) |
| Starts without Python, Docker, a database server, Ollama or an LLM | ✅ the release DMG smoke and the CI installs |
| Smoke on all three OSes: catalog, reading to completion, a passage search, open from a result, pause and resume of reading, quit and relaunch | ◐ Windows and Linux automated (with stand-ins for the dialog and the opener); macOS by hand from the release DMG |

## Acceptance checklist (§16): assessment

| Item | Assessment | Evidence |
| --- | --- | --- |
| Finds a book or paper by a phrase from its text, not just its name | ✅ | P1-16, P1-17; smoke "search finds a passage"; DMG smoke step 5 |
| Shows for every file whether its text is ready, waiting, needs OCR, is encrypted, only in the cloud, or couldn't be read, with a reason | ✅ | P1-04, P1-13; DMG smoke step 4 |
| Never writes to, moves or downloads an original while reading it | ✅ | the originals snapshot in every extraction test; P1-13 (no recall on Windows, dataless files refused on macOS) |
| Slows on battery, pauses at low battery, low space or quit, resumes where it stopped | ✅ macOS · ◐ Windows, Linux | P1-14 (simulated), P1-12, P1-22; smoke and DMG pause steps |
| The same content in several places appears once, with all its locations | ✅ | P1-06; Library › Duplicates |
| Search and browsing stay responsive while text is read | ✅ | P1-19 on the reference machine; 1k on CI |
| Installers on three OSes include the PDF library and start without Python or anything else | ✅ | release bundles in CI; the DMG smoke; worker hello from each installed bundle |
| Rescanning an unchanged library reads no text again | ✅ | P1-03; DMG smoke step 9 (nothing read again after relaunch) |
| Storage use by category, with an honest estimate of the remaining text | ✅ | `tests/storage.rs`; Settings › Storage in the smoke and the DMG smoke |
| Recorded test, benchmark, license and platform-smoke evidence for the three OSes | ✅ | this document and the files it links |
| Clearly distinguishes what works now from what is still to come (OCR, semantic search, AI) | ✅ | README capability table; the app says "Needs OCR" without claiming it; AI Models only tests connections |

## Fixed during this review

- **The smoke test didn't pause reading or open a search result**, though
  §14 lists both. It now pauses and resumes reading after the relaunch
  (checking the count stops) and opens the passage it found from the
  result.
- **The Windows smoke test failed at its first step in five of nine recent
  runs**, before the app opened: msedgedriver refused tauri-driver's
  connection. The smoke test now waits for the native driver's port and
  retries a refused session once, recording each retry in its report.
- **Two crash tests skipped the integrity checks** §12 asks for after
  every crash test: P1-11 (workers crashing and hanging) and P1-22 (a
  killed app). Both now end with `PRAGMA integrity_check` and FTS5's
  `integrity-check`, as P1-12 did.
- **The 30k extraction gate had no check that every file was read.** The
  extraction benchmark now records counts by status, and
  `--require-all-read` fails unless every file is "Text ready"; the opt-in
  evidence run (`just ci-remote evidence`) runs it on every OS.
- **Stale README copy.** After describing PDFium, rbook and usearch, the
  README still said "None of this is in the app yet"; PDFium and rbook have
  read text since Slice 1.3. It now says so, and that usearch comes with
  semantic search.
- **The DMG checklist expected one result for `Astronomy marker 60`
  without quotes.** Unquoted, the two documents with all three words come
  first and the page fills with "Some of your words" results, as decided
  in Slice 1.4; the product owner kept that behaviour, and the checklist
  now says so.

## Not verified (carried forward)

Each item is covered by the
[manual checklist for Windows and Linux](platform-validation.md#manual-checklist-for-windows-and-linux)
or is planned work.

| Gap | Platforms | Covered by |
| --- | --- | --- |
| Native folder dialogs; opening and revealing in the OS | Windows, Linux | Manual checklist (Data location, Library folders, Library, search, open) |
| Opening or revealing a file at a path over 260 characters | Windows | Manual checklist (Carry-overs) |
| Credential Manager / Secret Service with a real key | Windows, Linux | Manual checklist (AI services) |
| Real battery, AC and drive changes during reading | Windows, Linux | Manual checklist (Reading text, Carry-overs) |
| 30k search latency (P1-19) | Windows, Linux | 1k on CI; the reference machine is macOS |
| Catalogs created before Slice 1.7 keep 4 KiB pages | all | Phase 4 maintenance tools |
| Installer experience: SmartScreen, Gatekeeper, uninstall | all | Manual checklist (Installers) |
| Placeholder app icons | all | Before any public build |

## Carry into Phase 2

Taken up by the [Phase 2 implementation specification](phase2-spec.md)
(2026-11-09). From Phase 1's measurements and decisions, not new
requirements:

- **Semantic search** (the product spec's Phase 2): embeddings of the
  stored passages, with usearch as the vector engine
  ([ADR 0008](adr/0008-extraction-and-vector-engines.md)), ranked alongside
  the full-text index rather than replacing it.
- **Jobs.** Embedding is a third job kind on the same runner, with the
  duty cycle (rests pooled to 100 ms) and the battery policy. One
  extraction worker per library reads 777 pages a second at the Run duty
  cycle, so a second worker isn't needed for extraction; embeddings will
  need their own measurement.
- **Storage.** The catalog is 1.62× the text; vectors will add to it, so
  Settings › Storage and the indexing estimate need a vector category.
  Compressing passages remains an option.
- **Re-reading.** Per-format reader versions and the stay-searchable
  re-read (ADR 0011) are the pattern for re-embedding when a model
  changes.
- **Before Phase 2 starts:** choose the default embedding model and its
  size budget (downloads are outside Phase 1), and the share of the
  evaluation questions semantic search must answer that full-text search
  doesn't. Decided 2026-11-09: Qwen3-Embedding-0.6B through a connected
  provider (no downloads by Atlas), and 80% of a new vague-question set.
