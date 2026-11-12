# ADR 0021: Diagnostics: a log of typed events and a counts-only report

- **Status:** Accepted (Phase 4, Slice 4.1). Amends
  [ADR 0017](0017-trails-and-privacy.md) ("Atlas has no log") and
  [ADR 0009](0009-jobs-and-extraction.md) (the worker's hello).
- **Date:** 2026-11-12
- **Context documents:** [Phase 4 spec](../phase4-spec.md) §5.8, §7.1,
  §7.9, §8, §12 (P4-01 to P4-04); [ADR 0001](0001-local-ipc-no-server-single-data-root.md)
  (the single data root); [ADR 0015](0015-ask-atlas-answers-citations-injection.md)
  and ADR 0017 (what may never be written)

## Context

Until Phase 4, Atlas kept no record of what it did. `logs/` was created in
every library and left empty. A failure was seen at most once, on the
screen. That was a sound choice while every check ran on fixtures. Phase 4
meets the owner's real collection, which the coding agent never sees. The
evidence from it has to reach the close-out some other way.

Four facts shaped the design:

- **The privacy rules are strict and already tested.** Since Phase 2, keys,
  request bodies, words searched, questions, answers, notes and excerpts
  are never written outside the knowledge store (P2-20, P3-16). A general
  text logger can't keep that promise: one `format!` with a title in it
  breaks it, and no test catches every such call.
- **The work is long and runs in the background.** A log must never fail
  a job or slow it.
- **Several things are app-wide** (the resource governor, the panic hook),
  while each log belongs to one library.
- **Tauri doesn't expose a command's response** to the app (tauri 2.12).
  The one place an error meets the window is `impl Serialize for
  AtlasError`, and that place doesn't know the command's name.

## Decision

### A closed set of events

`services/diagnostics/event.rs` defines `Event`, a closed enum serialized
as one tagged JSON object. Every field is an integer, a duration, a
boolean, or a value of a closed enum such as `JobKind`, `PauseReason`,
`ErrorCode` or `Crash`. No field can hold text made while Atlas runs.

The one exception is a panic's source file, a `SourceFile`. It can only be
made from a panic's location, and only Atlas's own files are kept, as
`src/…`. Any other file becomes `dependency`, so no build machine's path
reaches the log. The privacy rule is kept by the type, and P4-02 checks it
on a real session.

What is recorded:

- Atlas starting.
- A library opening, with the schema it had if it was migrated, and the
  knowledge store's state. A library closing, with how long it was open.
- Each job starting and ending, with its state, time, items and errors.
- Each pause and resume, by the user or by the policy, once per change.
- Workers starting, crashing (by kind) and being refused.
- Each change in the policy's decision.
- Snapshots taken, failed or skipped for space.
- Each error returned to the window, by code.
- Each search: its time, its hits, and whether meaning was used.
- Each answer: its time, its sources, the citations kept and dropped, and
  how it ended.
- The meaning index being loaded and let go.
- A panic, with its file, line and column.
- Events dropped, a report saved, the log cleared.

### The writer

`Diagnostics` is a cheap handle that every part of the library holds. The
workspace owns one; the app shares it with the governor and the panic
hook.

- `attach(root)` is called as soon as a library's lock is held, so a
  migration at open is logged. It returns a guard, kept as the open
  library's last field: everything else records into it as it closes.
- `record` puts a line on a bounded queue (1,024) for a thread of its own.
  A full queue drops the event and counts it. The writer then writes one
  `events_dropped` line.
- **Files.** `logs/atlas-<UTC date>.jsonl`, a new one each UTC day and past
  5 MiB (`-1`, `-2`, …). The folder is kept under 50 MiB by removing the
  oldest files.
- Only names of that pattern are read, changed or removed.
- **A failed write** stops the log until the library opens again. Settings
  says so.
- **With no library open, nothing is written.**
- A panic is written at once on the panicking thread, past the queue,
  using `try_lock` so it never waits on a lock that thread may hold. The
  default hook still runs after it.

### The report

`report.rs` builds one JSON file (`reportVersion` 1) in
`exports/diagnostics/`. It holds:

- versions;
- the device check;
- the settings that are modes and switches, with services counted, never
  named;
- storage by category;
- counts of files, documents, pages needing OCR, passages, chunks, vectors
  per model (by position) and jobs;
- the knowledge store's counts, or its state only when it can't be read;
- the log's last 2,000 events.

It never holds a path, a name, a title, a model's or service's name, a URL
or a key. The report is written inside the data root, so it adds no
exception to ADR 0001. Revealing it is the user's step.

### The worker's hello (amending ADR 0009)

The supervisor now refuses a worker whose `Hello.protocol` differs from
the app's. That is a mismatched install. It is reported as
`Unavailable(ProtocolMismatch)`, not as a crash, so it isn't restarted ten
times. The extraction job ends with the code `worker_protocol`, and the
status line says Atlas couldn't start its document reader.

## Consequences

- **Errors are logged by code, not by command.** Naming the command would
  mean wrapping all 79 commands. The code, with the events around it, has
  been enough to place a failure. This deviation is recorded in the spec.
- **Recording costs about 0.2 µs** on the caller, and writing about 1.6 µs
  and 122 bytes per event (`measure_the_cost_of_recording`, M5 Pro).
  Events are per job and per change, never per file. A reading session of
  thousands of files writes tens of lines.
- **A burst of thousands of events in milliseconds drops most of them.**
  The drops are counted, which is the point of the bound. Nothing in Atlas
  records that fast.
- **Models are counted, not named.** `profile_label` is free text a user
  typed, and model names can carry the same. The spec's "model names are
  configuration and are included" is withdrawn.
- **Logging the meaning index found one cause of P3-18.**
  `VectorRegistry::loaded` used `try_lock` on the same lock `get_or_open`
  takes. An embedding job fetching its index at that moment made
  suggestions (and meaning search) see "not loaded". The index and the
  lock an open holds are now apart.
- **It wasn't the only cause.** After the fix, the second P3-18 test's
  first call came back empty once in a full local run, and never again in
  14 runs alone or under load. That test now waits for its first
  suggestions too, and if it ever fails it prints the model, the index and
  the seed count.
- **Later slices add events of their own**, in the same closed form: OCR's
  pages, rescans, checks, backups and moves.
