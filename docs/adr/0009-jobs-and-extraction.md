# ADR 0009: General jobs, and the extraction job

- **Status:** Accepted. Jobs built in Phase 1 Slice 1.1, the worker
  process in Slice 1.2, extraction in Slice 1.3.
- **Date:** 2026-11-08
- **Context documents:** [Phase 1 spec](../phase1-spec.md) §6.1, §7.1, §9;
  [Phase 0 spec](../phase0-spec.md) §6 ("migrate `scan_sessions` into the
  general table … when a second job kind exists") and §9 (abstraction
  budget); [ADR 0005](0005-inventory-durability.md) (scan durability);
  [ADR 0008](0008-extraction-and-vector-engines.md) (extraction libraries)

## Context

Phase 0 had one kind of background work, a scan, and its job record was the
scan session itself. Phase 1 adds text extraction, the second kind. The
Phase 0 spec asked for the scan table to become the general jobs table at
that point, instead of running two job systems side by side, and for
`JobProcessor` to become a trait only when a second kind exists.

## Decision

**One table, `jobs`, with a `kind`.** Migration 0007 renames
`scan_sessions` to `jobs` and adds `kind TEXT NOT NULL DEFAULT 'scan'`.
Every existing row becomes a scan with its state, counters and frontier
intact. Both kinds are per source root, so they share the columns that
pausing and status need (state, pause and throttle reasons, priority,
retries, timestamps). The scan counters stay as plain columns and are
unused by other kinds. `scan_directories.session_id` and
`file_instances.last_seen_session_id` keep their Phase 0 names.

**No CHECK constraint on `kind`.** The migration runner applies each
migration in a transaction with foreign keys on, so adding a value to a
CHECK would mean rebuilding `jobs`, and dropping it would cascade-delete
the scan frontier. The Rust `JobKind` enum is the guard instead: a row with
an unknown kind fails to decode rather than being treated as a scan. The
same finding moved the extraction columns of `file_instances` out of 0007
(see the Phase 1 spec's revision notes).

**One runner thread per kind, per open data root.** `JobRunner` owns the
loop Phase 0's inventory worker had: pick the next runnable job of its
kind (running first, then policy-paused, then queued, by priority and age),
run it, wait when idle, back off on database errors, stop when the root
closes. Runners are kept by kind, so two runners can never pick the same
job. When the root closes, every runner is told to stop before any is
joined, so shutdown takes as long as the slowest runner, not the sum.

**`JobProcessor` is a trait.**

```rust
pub trait JobProcessor: Send {
    fn idle_wait(&self) -> Duration;
    fn started(&mut self) {}
    fn run(&mut self, job: JobRow) -> Flow;
}
```

A processor is built per open root by a factory that receives the catalog
writer handle, the data root and the stop signal, and is built on its
runner's thread, so it is dropped there too. The scan is `ScanProcessor`,
Phase 0's worker body unchanged. The workspace registers one factory per
kind (`with_inventory` for scans, `with_job_processor` for others); tests
register a scripted extract processor to prove two kinds run together.
`SchedulingPolicy` stays the concrete `ResourcePolicy`: there is still one
policy.

**Control is generic.** Pause, resume and cancel act on any job ID with the
Phase 0 transitions and error codes, and wake every runner. Enqueueing is
idempotent per source root and kind. Status stays per kind: the
cataloging overview, the folder lines and the Library's "scanning" flag
consider scans only.

**The extraction worker is a mode of the app executable** (Slice 1.2).
`atlas --extract-worker`, checked first in `main.rs`, starts no window,
plugin or workspace; PDFium is loaded only there, never in the app process.
The spec allowed a separate `[[bin]] atlas-extract` sidecar if starting the
GUI executable in worker mode took more than about 300 ms to reach `hello`
on any OS. It takes about 15–20 ms on the M5 Pro (debug, release, from the
signed `.app` and from a DMG copy) and comparable times on the CI runners
([benchmarks](../benchmarks.md#extraction-worker-start)), so there is no
sidecar. The first start of a newly signed or copied binary on macOS takes
240–390 ms once, while the OS checks the signature; a sidecar would be a
second binary paying the same cost.

**`hello` proves PDFium works.** Before its first line the worker finds the
library, binds it, and extracts a two-page PDF compiled into the binary,
checking its page count and marker phrase. `hello` carries the self-test
result; `unavailable {reason}` (`not_found`, `load_failed`,
`self_test_failed`) and exit code 3 replace it when PDFium can't be used.
The spec's Slice 1.2 gate said "extracts one PDF" from the bundle; the
self-test does that without adding the `extract` request before Slice 1.3.

**The protocol stream is protected.** The worker keeps a duplicate of
stdout for JSON lines and points descriptor 1 (and on Windows the standard
output handle and the C runtime's descriptor 1) at stderr before loading
PDFium. Reasons are fixed codes; library messages go to stderr only.

**Signing.** With the hardened runtime and an ad-hoc signature, macOS
library validation refused the bundled PDFium ("different Team IDs": an
ad-hoc signature has none). The app is therefore signed with
`com.apple.security.cs.disable-library-validation` (`src-tauri/Entitlements.plist`),
the spec's planned fallback. Signed with a Developer ID, the app and the
library would share a Team ID and the entitlement could be dropped; it is
reviewed with notarization.

**Documents are content; files point at them** (Slice 1.3). A document is
the BLAKE3-256 of a file's bytes. It holds the extraction outcome, the
metadata and the text (`passages`: one per PDF page, or per EPUB chapter
piece of at most 4,000 characters). A file instance links to it once
hashed, so copies and moved files are read once. Migration 0008 creates
`documents`, `passages` and the `file_instances` columns; the spec put
`passages` in 0009 with full-text search, which would have discarded 1.3's
text, so only the FTS5 index waits for Slice 1.4. The per-file state keeps
only what blocks a file before hashing (`skipped_cloud_only`, `unreadable`,
`hash_failed`) or `linked`; the document carries the rest. "Reading…" is
the file the active job reports, not a stored state.

**Each document commits in one transaction**, guarded by the job still
running: the document row (an upsert by hash), all its passages, and the
link. A kill leaves the whole document or nothing (P1-12), and a pause
lands no late writes.

**The extract job walks its root's pending files with a rowid cursor.** A
file whose size or mtime differs from the catalog stays pending for the
next scan instead of being retried in a loop. A file that crashes the
worker is retried once with a fresh worker; the second crash marks it
`worker_crashed`, and the job goes on. Ten restarts in one job back off
like a scan's database failures.

**Safety rules run before parsers.** The worker sniffs signatures (a text
file named `.pdf` never reaches PDFium) and pre-scans EPUB zips before
rbook reads anything: DRM in `META-INF/encryption.xml` (font obfuscation
alone is not DRM), entries that escape the archive, and entries over
32 MiB or archives over 256 MiB (declared sizes; nothing is inflated). The
Slice 0.9 spike checked for DRM only after reading every chapter, and let a
zip bomb reach 352 MB; both are fixed. The unreadable-text rule from the
spike's scorer marks scans and garbled fonts `needs_ocr` with no passages
stored.

**Supervision.** The app spawns the worker (`WorkerCommand`, injected so
tests can use the Cargo-built binary), reads frames on a thread, kills it
after 30 s without a frame or above `min(1 GiB, 15% of RAM)` resident, and
starts a new one for the next request. Dropping the supervisor kills the
worker, so it never outlives the open library.

**No worker outlives a killed app** (Slice 1.7, P1-22). A worker notices a
closed pipe only at its next read or write, so one hashing a large file,
inside a long PDFium call or hung would outlive an app that was killed,
crashed or force-quit, with nothing left to enforce the timeout or the
memory cap. The app passes its process ID in `ATLAS_WORKER_PARENT`, and a
watchdog thread in the worker ends the process as soon as that parent is
gone: on Unix when the worker is reparented (checked every 250 ms), on
Windows when a wait on the parent's process handle returns. It ends the
process with `_exit` (`TerminateProcess` on Windows), running no exit
handlers, because PDFium may be in the middle of a call on the main thread.
On Windows every worker also joins a Job Object with
`KILL_ON_JOB_CLOSE`, whose handle the app never closes, so the kernel ends
the worker when the app ends even if the watchdog never runs. On quit the
app shuts the workspace down even if its lock was poisoned by an earlier
panic.

**Policy.** Extraction shares the scan's resource governor (the same
battery hysteresis) and maps its mode to a duty cycle rather than a rate:
50% busy on Run, 20% on Slow, paused on Paused. `ATLAS_EXTRACT_UNTHROTTLED`
(debug builds) removes the rest. A finished scan with pending files queues
an extract job in its own transaction and wakes the extraction runner
through a waker shared by all runners of the root; opening a library
queues one for every root with pending files.

## Consequences

- One state machine, one recovery path (`requeue_interrupted` on open), and
  one set of commands for every kind of background work.
- A catalog opened by a Phase 1 build is schema 7 or later; Phase 0 builds
  refuse it as newer, without writing to it.
- Later kinds (OCR, embeddings) need a processor and a factory, not a
  migration of the jobs table.
- The scan thread is now `atlas-job-scan` (Phase 0's was
  `atlas-inventory`); each kind's thread is `atlas-job-<kind>`.
- Verified by P1-01 (a schema-6 catalog with a user-paused scan upgrades
  with its frontier intact) and P1-02 (a scan and a scripted extract job
  run together, and each pauses without touching the other), in
  `src-tauri/tests/jobs.rs`.
- Every Rust build needs the pinned PDFium (`just setup`), so Atlas builds
  only on the platforms it is pinned for: macOS arm64, Windows x64 and
  Linux x64.
- A document is deleted only when its last file is purged; an orphan
  sweep is not built yet (Phase 1 doesn't purge automatically).
- One worker per open library, for all of Phase 1 (product-owner
  decision, Slice 1.7). `ATLAS_EXTRACT_WORKERS` is not built: one worker
  reads 514 pages a second at the Run duty cycle on the 30k text
  fixture on the reference machine (646 on the 1k one), more than twice
  the 200 pages/s target
  ([benchmarks](../benchmarks.md#text-extraction-slice-17)), and a second
  needs parallel dispatch, a new crash and cursor scheme and care with
  identical files. Revisit when a later phase (embeddings) needs more.
- Verified by P1-22 (`tests/extraction_crash.rs`): a child app is killed
  while its worker is reading, and while its worker hangs inside a
  request; the worker is gone within seconds on every OS, and relaunching
  reads the rest.
- Verified by P1-03 … P1-14 (`tests/extraction.rs`,
  `tests/extraction_crash.rs`) and `tests/supervisor.rs`, against the real
  worker.
