# ADR 0014: The embedding job

- **Status:** Accepted (Phase 2, Slice 2.3)
- **Date:** 2026-11-09
- **Context documents:** [Phase 2 spec](../phase2-spec.md) §5.2, §5.6, §5.7,
  §7.2, §7.3; [ADR 0004](0004-resource-policy.md) (resource policy);
  [ADR 0007](0007-provider-connections-and-secrets.md) (connections, keys);
  [ADR 0009](0009-jobs-and-extraction.md) (jobs);
  [ADR 0012](0012-model-roles-and-capability-tests.md) (roles);
  [ADR 0013](0013-embeddings-durable-index-disposable.md) (storage, index)

## Context

Slice 2.2 gave vectors a durable home and the index a crash-safe file.
Something has to fill them: send the library's text, chunk by chunk, to the
embedding role's service and keep the answers. The 30k-file fixture is
about 0.6–0.9 million chunks, hours to a day of a local model's time. So
the job runs in the background for a long time, on laptops and on battery,
against services that may be slow, busy, down or reconfigured, and it must
never send a chunk twice, never leave half a document, and never get in
the way of searching.

## Decision

**One job per library folder, a third job kind** (`JobKind::Embed`,
`services/embedding/job.rs`), on the jobs framework of ADR 0009: the same
state machine, Pause, Resume and Cancel, and a throttled status event
(`embedding-status`). Migration 0013 adds its counters to `jobs` (chunks
and characters sent, time waiting on the service). The version bump also
makes a Slice 2.2 build refuse a library whose jobs it can't read.

**What it embeds:** documents with text and an available file in the
folder that the active model hasn't finished (no row, or a failed row
under three attempts), in rowid order, a few at a time. Files without text
are never waiting for meaning.

**Per document, one transaction.**

1. Read its passages and a fingerprint (its update time and passage ids).
2. Chunk the passages, and look up which hashes already have a vector.
3. Send only the misses, once each, in requests of at most 16 chunks or
   16,000 characters.
4. Write the chunks, the new vectors and the document's state together,
   on the writer, guarded by two checks: the job is still running, and the
   fingerprint is unchanged.

A re-read in between makes the document be skipped, not wrongly marked
done; it waits again. After the commit, the chunks join the in-memory index
and its seq advances.

Vectors already received for the document in hand are kept across retries,
so **no chunk is requested again after an answer**. Tests check this across
failures, a cancel and a SIGKILL.

**The backend** (`services/embedding/client.rs`):

- OpenAI-compatible `POST {base}/embeddings`, read by `data[].index`; Ollama
  `POST {base}/api/embed`.
- Connect 3 s, total 90 s, at most 8 MiB per answer.
- The key goes only to the profile's own address, with no redirects or
  proxy (ADR 0007).
- Each vector is checked for its length and normalised to unit length.
  Every request is also checked in the job: a vector of another length
  means the model changed.
- A request runs on the async runtime while the job's thread waits in
  100 ms steps, so a library closing or the app quitting abandons it at
  once.
- Bodies are never logged, and errors carry no text and no key.

**Policy.** The resource policy of ADR 0004 applies, with one difference
for this kind: **on battery the job pauses** where extraction only slows
(product spec §9: bulk embedding pauses on battery). This adds the pause
reason `on_battery`. Otherwise the job uses the Run duty cycle 0.5 and Slow
0.2 (power unknown, system busy), measured on the time spent waiting for
the service. Text comes from the catalog, so an unplugged library folder
doesn't pause it.

**Failures.**

| Outcome | What the job does |
| --- | --- |
| Network, timeout, 429, 5xx | Back off 1, 2, 4 … 64 s, ±20%, never shorter than Retry-After (at most 300 s). After six in a row, `failed_retryable` with `provider_unreachable`: "Can't index meaning · AI service not reachable", with **Retry** (Resume). On its own it is queued again after 1, 2, 4, 8, then 16 minutes. |
| 401/403 | `failed_terminal` with `provider_auth` |
| Model not found | `failed_terminal` with `model_not_found` |
| Vectors of another length | `failed_terminal` with `model_changed` |
| 400/413/422, or a wrong count | Split the request into single chunks. A chunk refused alone marks its document failed (one attempt more, `chunk_rejected`), and the job moves on. After three attempts the document counts as refused, not waiting. |

**Saving the index:** after 10,000 changes or 5 minutes, and when the job
pauses, fails, completes, is cancelled or the library closes. Tombstones
left by re-reads on the extraction thread are applied before each save.
Saves are staged in `indexes/vectors/staging/`, not `tmp/`, which Clean
caches empties. A saved file with more than 20% removed slots is rebuilt
into a fresh one when it is next opened (compaction).

**Model change** (`services/embedding/models.rs`):

- **What a model is:** the embedding role's service and model, at its
  dimensions and chunker version. Any change makes a new row, `building`,
  which the job fills while the newest `ready` row keeps serving. A first
  model serves while it fills.
- **Abandoned models:** a building model the role no longer names becomes
  `failed`.
- **When the building model is finished** (no embed job active, nothing
  waiting in any folder), it is validated against the model it replaces:
  - its coverage is at least the old one's (failed-for-good documents count
    as covered);
  - 50 random chunks find themselves, or an identical text, nearest;
  - its file saves.
- **Then** the new model becomes `ready` and the old one `retired`, or, if
  validation fails, the new one is `failed` and the old one keeps serving.
- **Cleanup:** retired and failed models' rows are deleted in batches of
  5,000, then the row, then their files. Index files of models the catalog
  no longer has are swept.
- **Holding a change:** a folder whose job the user paused or cancelled
  holds the change until it is resumed or **Index meaning now** is pressed.
- **In Settings:** choosing **Use** for a different embedding model when an
  index exists asks first: "Atlas will build a new meaning index with this
  model. The current one keeps working until the new one is ready."

**When jobs are queued:**

- when extraction completes a folder;
- when the library opens;
- when the embedding role changes, or a service is saved or deleted;
- when embedding through an internet service is allowed;
- on **Index meaning now**.

A job is queued only when it could run: a role is set and its service may
be used. "Needs an embedding model" and "needs your permission" are flags
on the status, not job states. Atlas works without AI, so the first is
said only to someone who added an AI service.

**Internet services need permission.** Settings › Privacy has one checkbox
in Slice 2.3: "Allow embedding my library through an internet service",
off. It is shown with the volume of text it would send. Without it, an
embedding role on an internet service (the three-way label of ADR 0012)
runs nothing, and the status line says "Indexing meaning needs your
permission". Services on this computer or another one need no agreement.
The disclosure for Ask Atlas and "Disable all external AI" are Slice 2.6.

**Catalogs created before Slice 1.7 are converted first**
(`services/catalog/convert.rs`). A catalog with 4 KiB pages holds one 2 KB
vector per page: 4,294 bytes per vector instead of 2,529 (Slice 2.2).

- **When:** when such a catalog has an embedding role and no vectors, the
  library's open converts it, while its lock is held and no other
  connection exists.
- **How:** `PRAGMA page_size = 16384`, `VACUUM INTO` a staged copy, check
  it, then rename it over the catalog.
- **If interrupted:** a kill before the rename leaves the old catalog
  untouched, and the rename is atomic. The crash test kills mid-conversion.
- **Without room:** with less free space than 1.2 times the catalog, it
  stays as it is.
- **Triggered by:** choosing a first embedding model, which reopens the
  library. The UI says it may take a minute.

**Status.** `get_embedding_status` and the event give an
`EmbeddingOverview`:

- the job to show, with its counters;
- files with text indexed, waiting and refused, overall and per folder;
- the model;
- whether a new index is building;
- the two flags;
- whether text is still being read.

The status bar's third line, the per-folder line with **Index meaning
now**, and Home's note use the copy of phase2-spec §5.2. Settings ›
Storage gains **Meaning index** (its catalog rows plus the index files)
and an estimate: the text waiting ÷ 1,050 characters per chunk × bytes per
chunk, ±25%.

## Consequences

- Search (Slice 2.4) reads the serving model from `models::serving` and
  its index from the library's `VectorRegistry`, and embeds queries
  through the same backend.
- The job's throughput is the service's: measured on oMLX by the owner in
  this slice (`docs/benchmarks.md`), and per dimension count in 2.4.
- The conversion rewrites a pre-1.7 catalog once (time about proportional
  to its size). It makes text search's 16 KiB-page saving of Slice 1.7
  reach old libraries too.
- ADR 0004 gains its first per-kind rule (the battery pause); ADR 0013's
  deferred items (save cadence, compaction, model change, retirement,
  stray files) are done here.
- Verified by:
  - **P2-05** (`tests/embedding.rs`, `tests/embedding_crash.rs`):
    everything once, identical text once, counts, cancel and Index meaning
    now, SIGKILL mid-job then relaunch, and the index equal to the catalog;
  - **P2-06:** the battery pause while reading continues;
  - **P2-07** (`tests/embedding.rs`, `tests/embedding_http.rs`):
    - backoff, then a failure, then Retry, while search keeps working;
    - Retry-After, timeouts, cancelling;
    - a refused chunk, 401, a missing model, wrong dimensions;
    - every HTTP answer classified;
  - **P2-09:** a model change and retirement;
  - **P2-19:** the Meaning index category, its estimate, Clean caches;
  - **P2-20:** the key only to its host, no redirect followed;
  - `tests/convert.rs` and the unit tests of `job`, `client`, `models`,
    `storage` and migration 0013.

## Amendment (Phase 4, Slice 4.1)

The job walks a folder's documents in rowid order. A document read again
after the job passed it waits again, and the reading job's "queue
embedding" trigger sees this job still running and queues nothing. Before
finishing, the job now looks once more from the start if any document
behind it has never been tried by the model
(`embedding::db::fresh_behind`). A document that failed and may be retried
doesn't count: its retry stays the next job's. Found when P3-13 (the
resume scenario) left three re-read books waiting on the macOS CI runner;
`tests/embedding.rs` `a_document_read_again_behind_the_job_is_embedded_before_it_ends`
holds it.
