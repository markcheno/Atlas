# Phase 4 progress: where things stand

**As of 2026-11-15, `main` at `ead40fe`, all CI green.** The product
owner is using the app on their real library for a while, and may polish
what exists before new features. This page is the place to pick the work
up again: what is done, what is open, and how to continue.

The plan itself is [phase4-spec.md](phase4-spec.md) (§15 is the slice
list; its revision notes record every change of plan, newest first). Per
slice evidence is in [platform-validation.md](platform-validation.md),
measurements in [benchmarks.md](benchmarks.md), design in
[architecture.md](architecture.md) and the [ADRs](adr/).

## Done

| Slice | What | Tag | ADR |
| --- | --- | --- | --- |
| 4.1 | Diagnostics: the local log, the report, the panic hook; page counts for scanned PDFs | `phase4/slice-01` | [0021](adr/0021-diagnostics.md) |
| 4.1.1 | A stopped meaning job says why; Clear the log asks first | `phase4/slice-01.1` | |
| 4.1.2 | Explore spike: answer **go** on the owner's 5,088 books ([spike-explore.md](spike-explore.md)) | `phase4/slice-01.2` | |
| 4.1.3 | "Going slowly" says why | `phase4/slice-01.3` | |
| 4.2 | OCR spike: measured, then OCR deferred by the owner ([spike-ocr.md](spike-ocr.md)) | `phase4/slice-02` | [0020](adr/0020-ocr-engine.md) |
| 4.5 | Automatic rescans that write only what changed; the scan UI | `phase4/slice-05` | [0022](adr/0022-automatic-rescans.md) |
| 4.5.1 | One book in PDF and EPUB is one book, indexed and shown once | `phase4/slice-05.1` | [0023](adr/0023-one-book-two-formats.md) |
| 4.6 | Schedules: Automatic, Only while charging, Only between, Paused; full speed; the busy rule | `phase4/slice-06` | [0004](adr/0004-resource-policy.md) amended |
| 4.6.1 | Explore: the library as named areas on a map, drawn in the background and kept stable | `phase4/slice-06.1` | [0024](adr/0024-explore.md) |

After the `phase4/slice-06.1` tag, `main` has two follow-ups, both green:
- `de2dfb9`: a redraw keeps every old book in its area (fixed centres;
  more areas only as the library grows);
- `ead40fe`: a test (P4-28) no longer counts its own search query as
  re-indexing.

The build reports itself as Phase 4 slice 4.6.1 (`CURRENT_SLICE`), app
version 0.4.0. The catalog is at schema 18.

## Not done, and why

| Slice | State |
| --- | --- |
| 4.3 OCR | **Deferred** to a later version by the owner (2026-11-13): their collection has few scans (213 documents, about 45,000 pages). Phase 4's item 1 is struck through. |
| 4.4 Read this now | **Skipped for now** by the owner (2026-11-13). Its migration is reserved as 0019. |
| 4.7 Check library and repairs | **Next planned.** ADR 0025. |
| 4.8 Backups and restore | Planned. ADR 0026. |
| 4.9 Move Atlas data | Planned. ADR 0027. |
| 4.10 Scale | Planned: the lifecycle test, the 100,000-file runs, the release candidate for the owner's run. |
| 4.11 Evidence and close-out | Planned: version 0.5.0. |

**Ordering:** 4.7 to 4.9 must be done before the owner's run in 4.10
starts, because that run should not begin on a library that can't be
backed up (spec §15, ordering notes).

## Open with the owner

- **On the installed builds:** the checklists for
  [4.6](platform-validation.md#slice-46) (full speed, Stop, unplugging,
  Paused, an hours window) and
  [4.6.1](platform-validation.md#slice-461) (the first map, renaming, Show
  in Library, Find on the map, a redraw keeping its shape). The owner
  reported "looking real good" and is using it; record findings there
  when they come.
- **Polish before new features:** the owner may ask for changes to what
  exists. Treat each as a small slice or a fix, as below.
- **Explore candidates not built:**
  - names for areas from the chat model;
  - "what the library is made of" (blocks by folder and format).

  Also not chosen: trails on the map, a walk through neighbours, books
  that bridge two areas. All are in
  [spike-explore.md](spike-explore.md#the-answer-is-go).

## Known issues and things to watch

- **Windows P2-05** (`tests/embedding.rs`) once found nothing indexed after
  the job ended, on `main` after the Explore merge. It didn't recur in 10
  local runs or in any later CI run. If it comes back, look at
  `wait_idle` racing the job's queueing.
- **Release bundle jobs on macOS** sometimes wait for a runner and are
  cancelled; rerun them. It's capacity, not code.
- **Explore at 30,000 books:** a first map takes about 14 s and a redraw
  about 10 s, mostly the exact neighbour search. The owner's 5,088 books
  draw in about 2 s.
- **The busy rule's thresholds** (60% and 40% of all cores) are measured
  on one Mac only.

## How to resume

**The slice workflow** (as used throughout Phase 4):
1. Plan the slice with the owner's choices, and get the plan approved.
2. Branch `slice-NN` (e.g. `slice-47`) from `main`.
3. Code and tests.
4. Docs:
   - README status and feature table;
   - architecture;
   - an ADR if a decision is made;
   - benchmarks;
   - platform-validation (what was verified, plus the owner's checklist);
   - the spec's revision notes;
   - the justfile for any new repeatable command;
   - this page.
5. Run `just ci` locally. It runs fmt, clippy, every Rust suite, Vitest,
   typecheck and the bindings check; new bindings must be committed.
6. Commit, push, and start the full CI:
   `gh workflow run ci.yml --ref <branch> -f scope=full` (or
   `just ci-remote`).
7. When it's green, squash onto `main` as `phase4/slice-0N: …`, add an
   annotated tag of the same name, and push both.
8. Confirm `main`'s own CI (which adds benchmarks) is green. If it isn't,
   fix on a branch with full CI before pushing to `main` again.

**A fix to `main`** goes the same way:
- a `slice-…-fix` branch;
- full CI;
- squash onto `main` as `fix: …` or `test: …`, without a tag.

**The owner tests release builds**, never debug runs. A build's DMG comes
from its full CI run:

```sh
gh run download <run id> -n atlas-macOS-ARM64-release -D ~/Downloads/atlas-<slice>
```

**Practicalities:**
- **A second worktree** (for example to test an older commit) needs the
  vendored PDFium: symlink `src-tauri/vendor/pdfium` from the main
  checkout, and remove the link before removing the worktree.
- **The owner's library is never opened by the tools.** Owner-run
  measurements are justfile recipes that read a library they point at
  (e.g. `just spike-explore`), or reports they save (Settings ›
  Diagnostics).
- **Repeated test runs find flakes:** document ids are random UUIDs, so
  ordering-sensitive tests vary between runs. Before declaring a fix,
  loop a test 30 to 60 times.

**Where things live, for the most recent work:**
- **Resource policy and schedules:** `src-tauri/src/services/scheduler/`
  (`policy.rs`, `schedule.rs`, `busy.rs`, `duty.rs`, `watch.rs`) and the
  governor in `services/system/mod.rs`.
- **Explore:** `src-tauri/src/services/explore/` (`build.rs`, `watch.rs`,
  `app.rs`, `cluster.rs`, `layout.rs`, `hues.rs`).
  - Window: `src/views/Explore.svelte`,
    `src/lib/components/ExploreMap.svelte`,
    `src/lib/stores/explore.svelte.ts`.
- **One book, two formats:** `src-tauri/src/services/formats/`.
- **Rescans:** `src-tauri/src/services/inventory/` (`auto.rs`, `scan.rs`,
  `db.rs`).
