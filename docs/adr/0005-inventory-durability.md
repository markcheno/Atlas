# ADR 0005: Durable inventory

- **Status:** Accepted
- **Date:** 2026-11-07
- **Context documents:** [Phase 0 spec](../phase0-spec.md) sections 5.6, 7.1–7.3 and 11;
  ADRs [0002](0002-data-root-creation-locking-and-compatibility.md),
  [0003](0003-source-root-identity-and-overlap.md), [0004](0004-resource-policy.md)

## Context

Cataloging a lifetime library can take hours on battery. The app may be
quit, killed, or put to sleep at any moment, and folders may vanish when a
drive is unplugged or a cloud service goes offline. Many users keep books
in iCloud Drive with "Optimize Mac Storage" enabled, so many files exist
only as placeholders. The catalog must stay correct through all of this.
It must never duplicate a row or lose completed work. It must not report a
file deleted when Atlas simply couldn't see it, and it must never download
or modify a user's file.

## Decision

**The scan session is the job.** `scan_sessions` holds the state machine
from spec §7.3 and the counters. There is no generic jobs table until a
second job kind exists. All state changes are conditional updates
(`… WHERE state IN (…)`), so the worker and the user can never overwrite
each other. A policy pause can't replace a user pause, and a cancel can't
be undone by the worker. A batch commits only while its session is still
`running`: the commit's own counter update is conditional, so a batch
listed while the user paused or cancelled is dropped instead of landing
afterwards, and its folder is listed again on resume. Transitions and
commits share the single writer connection, so the check cannot race a
state change. (Added in Slice 0.8, after the first CI run on slower
hardware caught a late batch after a pause.)

*Amended 2026-11-08 (Phase 1, Slice 1.1):* the second job kind arrived, so
`scan_sessions` is now the general `jobs` table and a scan is a row with
`kind = 'scan'` (migration 0007, [ADR 0009](0009-jobs-and-extraction.md)).
Everything in this ADR still holds for scans. The conditional state changes
moved to `services/jobs/db.rs`, and the scan's batch commit and frontier
stay in `services/inventory/db.rs`.

**A persisted directory frontier.** `scan_directories` lists folders still
to scan for a session. Atlas lists a folder's entries in batches of up to
100, and each batch is one transaction. That transaction upserts files,
adds newly found subfolders to the frontier with `INSERT OR IGNORE`, and in
the folder's last batch marks the folder done. If work stops mid-folder,
the folder is listed again from the beginning; idempotent upserts keyed on
`(source_root_id, rel_path)` make that safe. Frontier rows are deleted when
a session ends. A very large folder interrupted part-way repeats its
listing; this is a known, measured cost.

**Recovery is a requeue.** When a root opens, sessions that were `running`,
`paused_policy`, or `failed_retryable` become `queued`; `paused_user`
stays paused. A folder that has never been scanned, such as one added
before cataloging existed, gets one queued session. A real `SIGKILL`
mid-scan is covered by `tests/inventory_crash.rs`.

**One worker, owned by the open root.** `OpenRoot` declares the worker
before the catalog and the lock, so closing a library stops and joins the
worker, then closes SQLite, then releases the lock. The worker writes
through a cloned `CatalogHandle`, so the single-writer rule still holds.
It never takes the workspace mutex, so user commands are never blocked
behind a scan.

**Policy and pacing.** Before work starts and every 2 seconds during it,
the worker probes the source root and asks the shared `ResourceGovernor`
(ADR 0004) what to do. If the root is unreachable, it pauses with
`source_root_missing` and never fails. Other pauses use the policy's
reason. Running work is paced by a token bucket at the policy's rate.

**Missing means provably missing.** At the end of a session, a file not
seen in it is marked `missing` only if the folder that should contain it
was listed successfully. Files under a folder that could not be listed
after 3 attempts, or anything beneath it, are left untouched. If the root
is unreachable, the session pauses instead of finishing. The spec says
"only after a scan completes successfully"; this is a deliberate
refinement. An all-or-nothing rule would mean one permission-denied folder
hides every deletion in the library forever. The refinement keeps the
spec's intent: a disconnect or permission error never makes files look
deleted.

**Classification by extension only.** `.pdf` and `.epub`, case-insensitive.
Signature sniffing is deferred to Phase 1. It would open every file, and
for cloud-offloaded files it would trigger a download.

**Cloud-offloaded files are cataloged, never downloaded.**

- macOS File Provider "dataless" files keep their name and size. The
  `SF_DATALESS` flag in `st_flags` marks them `cloud_only`.
- Legacy iCloud placeholders, `.Name.pdf.icloud`, are cataloged as
  `Name.pdf`. The size comes from the small property-list stub, and the
  document is never opened. When the user downloads the file, the next
  scan finds the real file at the same relative path. It's the same row,
  now `local`.
- Windows cloud placeholders (OneDrive Files On-Demand, iCloud for Windows)
  carry `OFFLINE`, `RECALL_ON_OPEN`, or `RECALL_ON_DATA_ACCESS` attributes
  and are marked `cloud_only`.
- Listing a dataless folder may make the cloud service fetch its listing.
  That call gets a 15 second timeout. A timeout is an ordinary folder
  failure: it is retried, and its subtree is protected from
  missing-marking.
- Opening a cloud-only file (Slice 0.6) will download it. The UI must say
  so, and Phase 1 extraction must treat cloud-only files as needing an
  explicit, policy-aware download.

## Consequences

- A forced kill loses at most the batch in flight, which is replayed.
  Rows that existed before the kill keep their IDs.
- Status counts come from live queries during a scan and from stored
  counters afterwards. Skip counters can over-count slightly when a folder
  is replayed.
- An iCloud folder that is offline shows as a folder failure, not as
  deleted files.
- Throughput is bounded by the policy rate: 500 entries per second
  plugged in and 100 when slowed (ADR 0004, retuned in Slice 0.8).
  Unthrottled, a warm 30,000-file tree takes about 1.2 seconds on an M5
  Pro ([benchmarks](../benchmarks.md)).
