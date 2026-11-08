# ADR 0009: General jobs, and the extraction job

- **Status:** Proposed. The jobs part is accepted and built (Phase 1,
  Slice 1.1). The worker process is accepted and built as a skeleton
  (Slice 1.2). The rest of extraction is recorded here when Slice 1.3
  builds it.
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
- Extraction-specific decisions (supervision, documents by hash, per-file
  status, the duty-cycle policy) are added to this ADR in Slice 1.3.
