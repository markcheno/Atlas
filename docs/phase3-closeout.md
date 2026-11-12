# Phase 3 close-out review

- **Date:** 2026-11-12
- **Against:** [Phase 3 spec](phase3-spec.md) v0.1 and its revision notes
  (through Slice 3.9): §1 (definition of done), §12 (required tests), §13
  (benchmarks), §14 (packaging, CI and release evidence), §16 (acceptance
  checklist)
- **State reviewed:** `main` after Slice 3.9 (tag `phase3/slice-09`,
  version 0.4.0), including the fixes listed under "Fixed during this
  review"
- **Sign-off:** the §16 checklist is the product owner's. This review gives
  the evidence and a recommendation; it doesn't tick the boxes.

## Verdict

Phase 3 meets its definition of done on macOS, and in CI with a scripted
model service on Windows and Linux. The gaps are documented, as §1 allows.

- **macOS arm64.** Verified end to end:
  - every required test;
  - the reference machine's benchmarks: the page view, and the knowledge
    store at 100,000 steps with import's memory;
  - the release DMG smoke with a session resumed, by the product owner
    against oMLX ([below](#packaging-ci-and-release-evidence-14)).
- **Windows x64 and Linux x64.** Verified in CI:
  - every required test, P3-23's link cases included (now required on
    Windows);
  - the page view and the knowledge store at 100,000 steps in the evidence
    run;
  - an automated click-through of the built app. Once Phase 2's steps
    have run, it checks that the search and the question are recorded,
    quits and relaunches, resumes the trail and opens its citation, saves
    a bookmark, shows a PDF page, and exports the trail to a scratch
    folder. It then checks the note on disk.
  - As in earlier phases, the click-through answers native dialogs and the
    opener through debug-only stand-ins, and uses a scripted model
    service. The gaps are listed below with the checklists that cover them.

The product owner's Phase 0 decision ("CI only, document gaps") stands for
Windows and Linux.

## Definition of done (§1)

| Item | Status | Evidence |
| --- | --- | --- |
| 1. Trails are recorded: searches, questions with their answers and sources, opens; pause, delete, pin, 90-day expiry | ✅ | P3-09, P3-10, P3-11, P3-12, P3-14, P3-16; the smoke's "a search and a question are recorded"; DMG smoke step 1 |
| 2. A session resumes after weeks, through moves, re-reads and a model switch; anything gone says so; any search or question runs again | ✅ | P3-13, the committed resume scenario (12 weeks, a moved and a renamed file, a re-read book, a deleted one, Search again and Ask again); the smoke's resume step; DMG smoke steps 4 and 5 (the same day: see "Not verified") |
| 3. Bookmarks and findings survive moves, re-reads and reader upgrades, and are never removed for space | ✅ | P3-15, P3-08, P3-13, P3-07 |
| 4. Interests and projects group them; nothing requires one; Home shows recent trails, collections and suggestions | ✅ | P3-17, P3-18; Vitest for Home's sections; DMG smoke step 6 |
| 5. A PDF citation or bookmark opens the page, the passage marked when found, in a killable process | ✅ | P3-19, P3-20 (hangs, memory, malformed files; no PDFium in the main process on macOS and Linux), P3-21; the smoke's page view step; DMG smoke step 3 |
| 6. Everything can leave and come back: Markdown to a chosen folder, JSON, an import that restores it | ✅ | P3-22, P3-23, P3-24, a generated store's import; the smoke's export step; DMG smoke steps 7 and 8 |
| 7. User data is never lost to Clean caches, a crash, a kill or a damaged catalog, and is snapshotted unasked | ✅ | P3-04 (a store Atlas can't read is left alone, and the library opens), P3-05 (SIGKILL during writes), P3-06 (snapshots, and a kill during one), P3-07 (Clean caches, low space, every job), P3-08 (the catalog rebuilt) |
| 8. Unit, integration and smoke tests, benchmarks, packaging and licence evidence for three OSes | ✅ | this document and the files it links |

## Required automated tests (§12)

Every test runs in every full CI run on macOS arm64, Windows x64 and Linux
x64. Slice 3.9's final run is
[43150017832](https://github.com/markcheno/Atlas/actions/runs/43150017832).

| ID | Where | Notes |
| --- | --- | --- |
| P3-01 | `tests/extraction.rs` | Files back from the cloud are read at the next rescan (no watcher) |
| P3-02 | unit tests in `services/answers/retrieve.rs` and `services/providers/recommend.rs` | The profile's margin decides `weak`; an unknown model gets 0.20 |
| P3-03 | `tests/refresh.rs`; measured by `tests/gutenberg_reread.rs` | Only marked EPUBs are read again; the measurement is in §13 below |
| P3-04 | `tests/knowledge.rs` | A store that can't be used is left byte for byte and the library still opens |
| P3-05 | `tests/knowledge_crash.rs` | A child process killed during step, bookmark and collection writes |
| P3-06 | `tests/knowledge.rs`, `tests/knowledge_crash.rs` | At most daily, 14 kept, a failed check removed, a kill mid-snapshot |
| P3-07 | `tests/storage.rs` | Clean caches empties the page cache and nothing of the user's |
| P3-08 | `tests/knowledge.rs` | Moved, renamed, copied, re-read, changed, folder removed, catalog rebuilt |
| P3-09 | `tests/recording.rs` | Exactly the steps §7.1 names |
| P3-10 | `tests/recording.rs` | Paused writes nothing; a read-only store never fails a search or an answer |
| P3-11 | `tests/trails.rs` | 29 and 31 minutes, New trail, Continue, a relaunch |
| P3-12 | `tests/trails.rs` | Unpinned, pinned, in a collection, expiry off |
| P3-13 | `tests/resume.rs` + `fixtures/knowledge/resume-scenario.json` | The resume scenario (§10) |
| P3-14 | `tests/trails.rs` | Deleting a step, a trail and all trails; findings stay whole |
| P3-15 | `tests/saved.rs` | Anchors built in Rust; one bookmark per place; the note's limit |
| P3-16 | `tests/recording_privacy.rs` | No words, questions, answers, notes or excerpts outside the store |
| P3-17 | `tests/collections.rs` | Items leave collections; collections never delete items |
| P3-18 | `tests/suggestions.rs` | Seeds, an opened document, Not for this, a missing file, no index; no model call |
| P3-19 | `tests/reader.rs` | The marker outlined where the generator writes it; a scan; the last page plus one |
| P3-20 | `tests/reader.rs` | Malformed PDFs, the limits, hangs and memory, the extraction job undisturbed, no PDFium in the main process (macOS, Linux; Windows has no cheap module list) |
| P3-21 | `tests/reader.rs` | From the cache, reused by content, the cap, a cloud-only file, Clean caches |
| P3-22 | `tests/export.rs` | Both link styles, safe names, the user's own note left alone, nothing deleted |
| P3-23 | `tests/export.rs` | Every refusal; the scratch tree outside `Atlas/` unchanged; link cases required on Windows since 3.9 |
| P3-24 | `tests/export.rs` | Round trip equal, the same store unchanged, the newer copy both ways, eight bad files refused whole, the size cap |

Vitest (225 tests) covers every Phase 3 screen state: the Trails screen and
its timeline, Saved, Interests and projects, Home's sections, the page
view, the Export dialog, and Settings' privacy and export sections.

## Benchmarks (§13)

| Requirement | Status |
| --- | --- |
| The knowledge store at 10,000 trails and 100,000 steps: the Trails list and its filter, a 50-step trail, recording's cost, a snapshot, size per step | ✅ every runner and the M5 Pro ([Phase 3 summary](benchmarks.md#phase-3-summary-slice-39)): list p95 0.7–5.3 ms, a 50-step trail 21.7–65.0 ms, recording 0.3–1.8 ms, about 3,980 bytes a step |
| The page view: first render, 1,200 px p50 and p95 with and without the passage, a cache hit, the worker's peak memory | ✅ every runner and the M5 Pro: p95 17.1 ms (M5 Pro) to 58.4 ms (Windows); a cache hit 0.4–1.5 ms; the passage outlined on 33 of 37 pages in the evidence run, 37 of 37 after this review's fix (M5 Pro) |
| Export: 1,000 notes as Markdown, the whole store as JSON, an import | ✅ 1,000 notes in 0.21–1.36 s; 247 MiB of JSON in 0.69–5.86 s; import applied in 11.7–46.1 s, peaking at about 2.4 times the file |
| The carry-over: the Gutenberg books re-read and re-embedded (how many chunks changed) | ✅ `tests/gutenberg_reread.rs` on every OS: 906 → 788 chunks, 12 embedded again, 776 kept |
| macOS first; Windows and Linux CI evidence before sign-off | ✅ evidence run [43140187034](https://github.com/markcheno/Atlas/actions/runs/43140187034) |
| Provisional targets | Recording under 5 ms ✅ (at most 1.8); the list p95 under 100 ms ✅ (at most 5.3); a 50-step trail under 200 ms ✅ (at most 65); a page under 500 ms p95 ✅ (at most 58); a snapshot under 2 s ✅ on the reference machine (0.72 s), ◐ 3.5–5.1 s on the CI runners |

## Packaging, CI and release evidence (§14)

| Requirement | Status |
| --- | --- |
| Licences: `png` as a direct dependency; GPL/AGPL flagged | ✅ [dependency-licenses.md](dependency-licenses.md) regenerated in Slice 3.9 with no changes: 586 crates, 7 MPL-2.0 entries flagged for review, no GPL or AGPL. `png` and `base64` became direct in Slice 3.7; both were already in the tree, under MIT OR Apache-2.0 ([packaging](packaging.md#licensing-obligations-of-the-bundles)) |
| Bundles: the growth since 0.3.0; no new native library | ✅ +0.7 to +1.1 MiB per bundle, +2.9 MB (macOS) and +4.3 MB (Linux) per executable ([packaging](packaging.md)); the page view uses the bundled PDFium |
| CI: the new tests in the full tier; the bench job adds `benches/knowledge.rs` and `benches/reader.rs` | ✅ every full run; the bench job on `main` since 3.7 (the page view) and 3.8 (the knowledge store at 10k) |
| Smoke: recorded, relaunch, resumed with its citation, a bookmark, the page view, an export | ✅ Windows and Linux (Slice 3.8; a debug-only hook answers the export folder dialog) |
| Release evidence: the DMG smoke on macOS against oMLX, with a real session resumed | ◐ the CI-built `Atlas_0.4.0_aarch64.dmg` (run 43138111316), run by the product owner against oMLX ([result](platform-validation.md#macos-arm64-slice-39-release-dmg-smoke)). The session resumed, the moved file's citation opened, and export and import worked. It found four problems, fixed below. The fixes are verified by tests and CI's smoke, not by a second DMG run |
| Starts without Python, Docker, a database server, Ollama or an LLM; without AI, trails, bookmarks, interests, the page view and export all work | ✅ the smoke's Phase 1 steps; the Rust tests of trails, saved items, collections, the page view and export run without a model; DMG smoke step 1 |

## Acceptance checklist (§16): assessment

| Item | Assessment | Evidence |
| --- | --- | --- |
| Weeks after a session I can open its trail and see what I searched, asked, was answered and opened, without saving anything | ✅ | P3-09, P3-13; the smoke; DMG smoke step 5 |
| A citation in an old answer still opens the right document at the right place, though I moved the file | ✅ | P3-08, P3-13; DMG smoke step 5 |
| When a file is gone, Atlas says so and still shows me the passage I saw | ✅ | P3-13; Vitest (RecordedSource); DMG smoke step 5 |
| I can pause recording, delete a step, a trail or everything, and nothing is recorded while paused | ✅ | P3-10, P3-14; DMG smoke steps 1 and 9 |
| I can bookmark a place and keep an answer or a passage with a note | ✅ | P3-15; the smoke's bookmark step; DMG smoke step 3 |
| I can group things under an interest or a project, and never have to | ✅ | P3-17; DMG smoke step 3 |
| Home shows where I left off and suggests things I haven't opened | ✅ | P3-18; Vitest; DMG smoke step 6 |
| A citation or bookmark in a PDF shows the page, the passage marked | ✅ after a fix | P3-19; the smoke, which since Slice 3.9 requires the outline to be drawn; DMG smoke step 3 found the page right but no outline, fixed below. 37 of 37 benchmark pages are outlined |
| I can export a trail, a finding or everything as Markdown into a folder I choose, and Atlas never overwrites my own notes there | ✅ | P3-22, P3-23; the smoke's export step; DMG smoke step 7 |
| I can export everything as one file and import it again | ✅ | P3-24; DMG smoke step 8 |
| Cleaning caches, a crash or a damaged catalog never costs me my trails and notes | ✅ | P3-04 to P3-08; DMG smoke step 9 |
| A book that was only in iCloud is read once I download it | ✅ (simulated) | P3-01 with the cloud-only fake; DMG smoke step 11 is optional (real iCloud) |
| Recorded test, benchmark, licence and platform-smoke evidence for three OSes | ✅ | this document and the files it links |

## Fixed during this review

- **The page view's outline was never drawn in the app** (the DMG smoke).
  - **The cause:** inline styles. Tauri's nonce in the CSP's `style-src`
    makes WebKit ignore `'unsafe-inline'`.
  - **Missed by the tests:** jsdom applies no CSP.
  - **The fix:** the outline is now an SVG. A Vitest check forbids inline
    styles in components, and the smoke requires a drawn outline.
- **The passage was looked for only by its start,** which a word
  hyphenated at a line end defeats. It is looked for in pieces now: 37 of 37
  corpus pages are outlined.
- **An open trail showed its sources as they were before a rescan** (the
  DMG smoke's move step). Trails fetches the open trail again when shown.
- **The battery pause read as stuck** (the DMG smoke). The line now says
  that indexing resumes when plugged in.
- **A library reopened at once could be refused as "in use".**
  - **Seen:** once in about six local runs of `tests/knowledge.rs`, whose
    tests run in parallel and spawn processes.
  - **The likely cause:** a process this one is starting can share the
    lock file briefly, until it has started.
  - **The fix:** the lock is now tried again for up to half a second
    before a library is called in use. 20 runs in a row then passed, and a
    real second Atlas is still refused.
- **How to export everything, and where it went, wasn't clear** (the DMG
  smoke). Settings now says **Export everything…**, the formats say what
  they are for, and **Show in Finder** follows an export.
- **The import cap held only about 100,000 steps.** A whole-store export of
  the benchmark's store was 247.5 MiB against the 256 MiB cap. It is 1 GiB
  now (the product owner's choice), and the benchmark records import's
  memory: about 2.4 times the file
  ([ADR 0019](adr/0019-export-and-import.md)).
- **P3-23's link cases could skip unseen on Windows.** The skip note went to
  the test's captured output, so Slice 3.8's run couldn't say whether they
  ran. CI now requires them there (`ATLAS_TEST_REQUIRE_SYMLINKS=1`), and
  they pass.
- **§13's carry-over measurement was never taken.** `tests/gutenberg_reread.rs`
  reads the corpus's Gutenberg books as reader version 2 did (a debug-only
  worker knob), then re-reads and re-embeds them, on every OS.
- **The evidence run was still Phases 1 and 2's.** `just ci-remote
  evidence` is now Phase 3's, in 8 to 15 minutes. The old run is
  `bench.mjs --evidence-phase2`.
- **`architecture.md`'s phase history stopped at Slice 3.3.**

## Deviations from the spec

Each one is recorded where it was made: the spec's revision notes and ADRs
0016 to 0019.

- **The knowledge store** (3.2):
  - **A store Atlas can't use never blocks the library.** It is refused,
    the library opens, and the store is left as it was.
  - **Changes are counted by triggers.**
  - **Snapshots are checked at open and hourly,** and taken at most daily.
  - **Version 2** (3.8) adds `updated_at` to trails and steps, so an import
    can keep the newer copy.
- **Recording** (3.3):
  - `start_trail` makes no empty trail.
  - `record_open` gains an action, and opens of the same document merge.
  - A note the user adds is written while paused.
  - **Atlas has no log**, so "logged once" is shown once.
  - `trail-changed` isn't coalesced.
- **The Trails screen** (3.4): tabs and actions arrived with their slices.
  §5.1's source sentences were completed.
- **Saved items** (3.5): the renderer sends a target (a file and a place,
  or a step). `SavedItem` is tagged `item`.
- **Collections and suggestions** (3.6):
  - documents opened in a trail are left out, not every document a trail
    touched;
  - a seed's vector is the mean of its documents';
  - the day's order is seeded by a hash.
- **The page view** (3.7):
  - **`RenderFailed`** carries reasons.
  - **The cache key** includes what was looked for, and a cached page shows
    for a file now only in the cloud.
  - **A tall page is scaled,** not refused.
  - **No PDFium module check on Windows.**
- **Export and import** (3.8, 3.9):
  - **A cancelled dialog is `null`.**
  - **Bookmarks** go one note per document.
  - **A JSON export is never replaced.**
  - **`lastFolder`** is kept outside the `Settings` DTO.
  - **Links and "Not for this" are only added** on import.
  - **A snapshot is taken before every import.**
  - **Notes are flushed once per folder.**
  - **The cap is 1 GiB.**
- **The carry-overs** (3.1):
  - **Only marked EPUBs are read again.**
  - **Notes inside a book stay.**
  - **A downloaded file is read at the next Rescan.**
  - **Hybrid fusion counts the best meaning match twice** (3.1.1).
- **Taken into 3.8 by the product owner:** `scripts/generate-knowledge.mjs`
  and `benches/knowledge.rs`.

## Found during this review, not fixed

- **Snapshots on the CI runners take 3.5 to 5.1 s** at 100,000 steps
  (a 380 MiB store), against 0.72 s on the reference machine and §13's
  2 s.
  - `VACUUM INTO` and the integrity check are disk-bound on shared runners.
  - Snapshots run in the background, at most daily and before an import,
    so nothing waits on one.
  - An incremental or page-level copy is the fix if real libraries get
    there.
- **The page view's outline can still be missed** where none of a
  passage's pieces match PDFium's page text. All 37 corpus pages match since
  this review's fix. The page still shows, with a note, and its text is
  below it.
- **Import holds the parsed file in memory:** about 2.4 times its size, so
  about 2.4 GiB at the cap. A streaming import is the way past that, if
  stores grow so large.
- **P3-18 once found no suggestions on its first call,** in a full local
  run (Slice 3.9); twelve runs of it alone passed.
  - Suggestions are empty while the meaning index isn't loaded, by design.
  - The test now waits for the first ones.
  - Why the index wasn't loaded at that moment isn't known.
- **P0-21 failed once on Windows in Slice 3.6** (closing took 2.28 s
  against its 1 s bound). It has passed in every run since, including the
  ones in this slice.

## Not verified (carried forward)

| Gap | Platforms | Covered by |
| --- | --- | --- |
| A real model service (the smoke uses the scripted one) | Windows, Linux | Manual checklist (AI services) |
| Credential Manager / Secret Service with a key | Windows, Linux | Manual checklist (AI services) |
| The export folder and import file dialogs by hand (the smoke's hook answers the folder dialog; import isn't in the smoke) | Windows, Linux | Vitest; P3-23 and P3-24 against the service; DMG smoke steps 7 and 8 on macOS |
| The Trails screen, the page view and the Export dialog with a screen reader | Windows, Linux | DMG smoke step 10 (VoiceOver) on macOS |
| A session resumed after real weeks | all | P3-13 with the settable clock (12 weeks); the DMG smoke resumes the same day |
| A real iCloud download read at the next rescan | macOS | P3-01 with the cloud-only fake; DMG smoke step 11 (optional) |
| Real battery during embedding | Windows, Linux | Manual checklist |
| The 30k fixture embedded through oMLX overnight | macOS | Projected at 2.5–5 hours (Phase 2); not run |
| Signing and notarization | all | Ad hoc on macOS, unsigned elsewhere |
| Why the graph search missed an index's first vector on the Linux runner | Linux; indexes over 20,000 vectors | Phase 2's exact search for small indexes; not reproduced |

## Carry into Phase 4

From Phase 3's measurements and decisions, not new requirements:

- **Phase 4's own scope** (phase3-spec §2): OCR, catalog backups, integrity
  tools, diagnostics, Move Atlas Data, and compressing stored passages.
  At about 4 KB a step, the knowledge store is 380 MiB at 100,000 steps;
  compression would help it too.
- **Snapshots of large stores:** incremental, or timed against the
  library's real size.
- **Import past 1 GiB:** streaming, if any library gets there.
- **The page view:** an EPUB renderer is still a later phase's reader.
- **Still deferred by the spec:** `atlas://` links, the "Check for newer
  recommendations" fetch, sync of trails and bookmarks (Phase 5), signing
  and notarization.
