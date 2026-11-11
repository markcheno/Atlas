# ADR 0017: Recording trails, and what that does to privacy

- **Status:** Accepted (Phase 3, Slice 3.3). Amends
  [ADR 0015](0015-ask-atlas-answers-citations-injection.md).
- **Date:** 2026-11-11
- **Context documents:** [Phase 3 spec](../phase3-spec.md) §5.2, §5.7, §7.1,
  §8, §12; [ADR 0016](0016-the-knowledge-store.md) (the knowledge store,
  anchors)

## Context

Phase 3 records research trails without being asked: every search the user
submits, every question with its answer and sources, and every source they
open from them. Weeks later the trail shows what they did, and each source
still opens at the right place.

That is the most personal data Atlas holds. Until now nothing a user asked
was kept anywhere (ADR 0015: "nothing is persisted"). Four facts shaped how
recording works:

- **The renderer is not trusted with content.** It must not be able to
  write a step's words, an answer, or an anchor. Phase 2's boundary has it
  send ids and short user text only.
- **Search and Ask look the same to Rust whatever the reason they ran.**
  Submitting a search, paging, a live refresh while text is read, and a
  filter change all call `search_documents` with the same query.
- **The answer thread never kept the full answer.** It drains text into
  `answer-stream` events every 50 ms.
- **Atlas has no log.** No logging framework exists; `logs/` is reserved
  and empty.

## Decision

### Rust records, from its own state

- **A search** is recorded by `search_documents` only when the renderer
  says `intent: submit`. The default is `refresh`, so paging, refreshes and
  filter changes never record, and nothing can record by accident. The
  same words and filters as the trail's last step update that step instead
  of adding one.
- **A question** is recorded when its answer ends: done, stopped, failed,
  or with nothing in the library to answer from.
  - When the answer starts, `ask` takes an `AskRecord`: the question, the
    filters with the folder's name as it was, each source's anchor with
    its excerpt as sent, `weak`, the meaning status, the model, and where
    its service was.
  - The answer thread keeps the text it delivered, `[n]` markers included,
    and records after its last event, so the screen never waits on the
    store.
  - A request refused before anything is asked (no chat model, external AI
    off, an invalid question, the disclosure) is not a step: no question
    was asked.
- **A source opened** is recorded by `record_open(fileId, via, action,
  place?)`, called by the renderer after the details panel, Open or Show
  in the file manager succeeded.
  - `via` says how the user got there: a result, a citation (with the
    answer's id and the number), Related, a suggestion, a bookmark, a
    trail, or the Library. The Library is never a step.
  - Rust builds the anchor from the catalog. It uses the passage the user
    saw: the recorder keeps the passages of the last page of results and of
    the last answer's sources, in memory only. Otherwise it uses the page,
    or the whole document.
  - Opening the same document the same way again (the details, then Open)
    adds the action to the last step.
- **Passage row ids never leave Rust.** `SearchHit` and `SourceUsed` carry
  one, skipped in serialization and in the bindings. `anchor::Place::Passage`
  uses it only while building: the anchor stores the ordinal (ADR 0016).

### Payloads

Each step is JSON in `trail_steps.payload`: `StepPayload` is `search`,
`ask`, `open` or `note`, each with `v: 1`. A question with eight sources is
about 4 KB. The excerpts are the cited sources' text as sent; uncited
sources are anchors without one. A search keeps its top five results, each
with a 300-character excerpt, enough to find it again after a re-read.
`step_documents` lists what each step touched: a search's results, a
question's cited sources, the opened document.

### Sessions

- A step joins the trail whose last step, or Continue, was under 30 minutes
  ago (`meta.current_trail`, `meta.current_trail_until`). Otherwise it
  starts one. The session survives a relaunch.
- **New trail** clears the current trail; the next step starts a new one.
  No empty trail is ever made, so `start_trail` returns the recording
  status, not a trail.
- **Continue this trail** makes an old trail current for 30 minutes.

### Pause, expiry and deletion

- **Pause** is `privacy.recordTrails`, in the library's settings file.
  Paused, nothing is written. Searching, asking and opening work as before.
  A note the user adds to a trail is their own act and is written anyway.
- **Expiry** (`privacy.expireTrails`, on): at open and from the store's
  hourly thread, at most daily. It deletes trails that are unpinned, in no
  collection, and whose last step is over 90 days old. Bookmarks and
  findings never expire, and a finding keeps its own copy of its sources.
- **Deletion** of a step, a trail or every trail is one transaction that
  also removes the trail from collections. A trail left with no steps is
  deleted.
- Titles are capped at 200 characters and notes at 4,000, in Rust.

### Failures never cost the user their search or answer

A step is one transaction. If it can't be written, the search or answer
goes on, and `RecordingStatus.failed` is set until a step is saved again.
The `trail-changed` event carries the change once, and the status bar says
"Atlas couldn't save this step". Atlas has no log, so the failure is
written nowhere. Diagnostics are Phase 4's.

### Privacy (amending ADR 0015)

ADR 0015's "nothing is persisted" becomes:

> An answer is persisted only as a trail step or a finding, in the
> knowledge store, and only while recording is on or because the user saved
> it.

The catalog, the job tables, the settings file and every other file in the
library still never hold a word searched, a question, an answer, a note or
an excerpt. Recording sends nothing anywhere. Search again and Ask again
(Slice 3.4) are ordinary searches and questions, under Phase 2's rules.

Test P3-16 checks this end to end. A child process records a session with
marker words, then the parent searches for them in three places, and finds
none:

- the child's stdout and stderr;
- every file in the library and its config except the store and its
  snapshots;
- every catalog table that isn't the documents' own text.

The same scan finds every marker in the store, so it would have caught a
leak.

## Consequences

- The renderer can only say what the user did, never what it was.
  A compromised renderer could record fake opens of real files, or start
  trails, but not write content.
- An answer stopped by closing the library is still recorded: the answer
  registry is dropped first and joins its thread while the store is open.
- Recording adds a few catalog reads (the anchors) and one store
  transaction to a search or an answer. Slice 3.9 measures it against the
  5 ms target.
- `list_trails` and `get_trail` exist from this slice. The Trails screen
  that uses them is Slice 3.4.

## Amendment: resuming (Slice 3.4, 2026-11-11)

- **Opening a source from a trail is a step,** reached `trail`, like any
  other open.
- **Search again and Ask again** continue the trail first, then run as
  ordinary searches and questions under Phase 2's rules. Their steps join
  the continued trail; the steps they repeat stay as they were.
- **A recorded answer is shown from the store,** never asked again, and is
  not a live region.
