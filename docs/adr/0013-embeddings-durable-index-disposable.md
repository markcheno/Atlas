# ADR 0013: Embeddings are durable, the vector index is disposable

- **Status:** Accepted (Phase 2, Slice 2.2)
- **Date:** 2026-11-09
- **Context documents:** [Phase 2 spec](../phase2-spec.md) §6, §7.3, §13;
  [ADR 0008](0008-extraction-and-vector-engines.md) (usearch, and the rule
  that the index is derived); [ADR 0009](0009-jobs-and-extraction.md)
  (`commit_document`); [ADR 0010](0010-full-text-search.md) (16 KiB pages)

## Context

Phase 2 searches by meaning. Each passage of extracted text is cut into
chunks, each chunk becomes a vector, and a usearch HNSW graph finds the
nearest vectors to a question. ADR 0008 chose usearch and set its rule:
usearch's `save` rewrites its file in place, and in the Slice 0.9 spike a
kill during a save left a file that no longer loaded (once in four kills).
So vectors need a durable home of their own, and the index must be
something Atlas can always throw away and build again.

Embedding is slow: about a day of a local model's time for the 30k-file
fixture (phase2-spec §7.2). Nothing may make Atlas embed a chunk twice:
not a re-read with the same text, not an identical copy, not a crash.

## Decision

**Vectors live in the catalog** (migration 0012) as little-endian f16 BLOBs
of unit length, keyed by the model and the BLAKE3 of the chunk's text.

- One transaction per document on the existing writer stores its chunks,
  its new vectors and its state, so a kill leaves whole documents only.
- Foreign-key cascades remove what a deleted passage, document or model
  owned.
- Storage accounting (`dbstat`) sees the vectors with everything else.
- `embeddings` is an ordinary rowid table with a unique `(model_id,
  text_hash)` index. A `WITHOUT ROWID` table keeps whole rows in its
  interior pages, and with 2 KB rows that took 4,294 bytes per vector in
  the benchmark instead of 2,534.

**Chunks** (`services/embedding/chunk.rs`, version 1):

- At most 1,200 characters, with about 150 characters of overlap.
- Cut at a paragraph break, then a sentence end (including CJK full
  stops), then a space, then a hard cut.
- A chunk never crosses passages, so it keeps its page or chapter.
- Offsets are character (Unicode scalar) offsets into `passages.text`.
- Chunk ids are `AUTOINCREMENT`, so a deleted id is never reused: the ids
  are the index's keys, and a tombstone must never remove a newer chunk.
- Chunks are shared by every embedding model. A document's chunks are kept
  when another model embeds the same plan, and replaced when the plan
  changes. There is one chunker version at a time; a new version re-chunks,
  and identical text reuses its vectors by hash.

**Content addressing**:

- Only chunks whose hash has no vector for the model are sent to the
  backend.
- An unchanged re-read re-chunks to the same hashes and sends nothing.
- `commit_document` now also deletes the document's `document_embeddings`
  rows when it replaces its passages (ADR 0009 amended). The document
  waits to be embedded again, and its old chunks cascade away, leaving
  tombstones.

**What the index holds**: the chunks of the documents the model has marked
done. `document_embeddings.seq` grows with every write for a model, and a
saved file records the last seq it holds (`indexed_through_seq`). A
chunk-id watermark would not work: a chunk created for one model can gain
another model's vector later, below that model's watermark.

**Tombstones** (`vector_tombstones`):

- Rows are sequenced, and an `AFTER DELETE` trigger on `chunks` adds one.
  The trigger fires for cascaded deletes too, which a test checks.
- Each model records the last tombstone its saved file applied.
- A save drains only the tombstones that every live (`building` or
  `ready`) model has applied.

**Saves are atomic and named by generation**:

1. Write the index to `tmp/`.
2. fsync it, rename it to `indexes/vectors/<model>-<generation>.usearch`,
   and sync the directory.
3. BLAKE3 the file.
4. Record the generation, the hash, `indexed_through_seq`, the vector count
   and the applied tombstones in one catalog transaction.
5. Remove the previous generation.

A kill at any step leaves the catalog naming a whole file that exists. The
crash test kills a saving child process eight times. Each time the kill
left a staged file mid-write or an unrecorded newer generation, and each
time the recorded file loaded.

- Searches continue during a save: it reads the graph under the read lock.
  Adds and removes wait, and saves run one at a time.
- At a million 1,024-dimension vectors a save takes 2.7 s on the reference
  machine.

**Opening** (`open_index`) verifies before it loads:

- The file must exist and match the catalog's BLAKE3. Unverified bytes never
  reach usearch's C++ loader.
- If it doesn't match, fails to load, has the wrong dimensions, or lacks
  more vectors than it holds, the index is rebuilt from the catalog from
  one read snapshot, on the given number of threads, and saved.
- Otherwise the file is reconciled from one read snapshot: it gains the
  chunks of documents written after its seq and loses the tombstoned keys.
- Generations the catalog doesn't name are deleted.

The in-memory index is authoritative and the file is a cache. Tests check
every path against a fresh build from the same catalog (P2-08).

**usearch options:** cosine, f16, connectivity 16, expansion 128 on add and
on search (the spike's measured setting), one vector per key. Capacity is
reserved in steps of `max(10,000, half the size)`.

**Paths:** usearch's own `save` and `load` take a narrow path, which Windows
reads in the ANSI code page. On Windows a path outside ASCII, or anywhere a
path isn't valid UTF-8, goes through `save_to_buffer`/`load_from_buffer` and
Rust's file I/O instead. That route costs a second copy of the index in
memory while saving or loading. A test runs both routes under a non-ASCII
library path on every OS.

**Dependencies:** `usearch` 2.26.6 (Apache-2.0, with `numkong` and `cxx`,
both permissive) and `half` 2.7 (MIT/Apache) for f16. On the reference
machine their C++ adds about 8 s to a clean release build (numkong 4.5 s,
usearch 2.2 s, cxx 1.1 s). The installers grew by 0.5 to 1.7 MiB
([packaging](../packaging.md)). Builds and every test pass on macOS
arm64, Windows x64 and Linux x64 (CI run 42724704393).

## Measured (reference machine, synthetic clustered vectors, 1,024 dimensions)

[Benchmarks](../benchmarks.md#vector-index-slice-22): at 1,000,000 vectors,
compared with ADR 0008's targets:

| Measure | Result | Target |
| --- | --- | --- |
| Recall@10 against brute force | 0.960 | ≥ 0.95 |
| Unfiltered p95, one thread | 0.82 ms | ≤ 100 ms |
| One folder (5%): recall, p95 | 0.989, 6.7 ms | ≥ 0.90, ≤ 150 ms |
| One folder (0.5%): recall, p95 | 0.994, 58 ms | — |
| Open → first answer (BLAKE3 check, load, reconcile) | 1.53 s | ≤ 2 s |
| File size ÷ raw f32 vectors | 0.536 | ≤ 0.6 (phase2-spec §13) |
| Catalog bytes per vector, 16 KiB pages | 2,529 | ≤ 2,500 (phase2-spec §13, provisional) |
| Build in memory | 49 s (18 threads) | — |
| Rebuild from the catalog (including its save) | 54 s | — |

The catalog is about 1% over the provisional 2.5 KB. The vector is 2,048
bytes, and the rest is its hash and model id twice (table and unique
index) plus the chunk row.

## Consequences

- *Amended 2026-11-09 (Slice 2.3):* the items below for Slice 2.3 are done
  ([ADR 0014](0014-the-embedding-job.md)). Pre-1.7 catalogs are converted
  to 16 KiB pages before their first embedding. Saves are staged in
  `indexes/vectors/staging/` instead of `tmp/`, which Clean caches empties.
  Compaction happens when a file is opened, and `apply_tombstones` keeps
  the in-memory index current with re-reads on other threads.
- **Catalogs created before Slice 1.7 have 4 KiB pages,** and a page holds
  only one 2 KB vector: 4,294 bytes per vector, 1.7× more. Slice 2.3
  decides between converting such a catalog to 16 KiB pages before its
  first embedding (`VACUUM` outside WAL), accepting the cost, or
  recommending 512 dimensions there.
- **Slice 2.3** (the embedding job) builds on this:
  - it calls `db::write_document` in its own guarded transaction, adds the
    returned chunks after the commit, and advances the index's seq;
  - it chooses when to save (10,000 changes or 5 minutes, and on pause,
    completion and close) and the rebuild thread counts under Run and Slow;
  - it compacts when tombstones pass 20% (usearch's file header counts the
    removed slots);
  - it handles retired models' rows and files, and sweeps files of models
    the catalog no longer has.
- **Slice 2.4** adds the filters' `ChunkMeta`, an exact search for small
  scopes, and Matryoshka truncation (`vector::truncate_normalize`) if 512
  dimensions win.
- The index can always be rebuilt, so its failures cost time, never data.
  A rebuild at a million vectors takes about a minute on the reference
  machine and longer on smaller ones.
- Verified by P2-04 (chunks map back to their pages; stable hashes; nothing
  re-sent; re-reads tombstone and re-queue; identical text embedded once)
  and P2-08 (built, saved, loaded; reconciled; damaged, missing,
  mismatched or stray files; tombstones across models; non-ASCII paths on
  both I/O routes; searches during saves; kills during saves), in
  `tests/vectors.rs`, `tests/vectors_crash.rs` and the unit tests of
  `chunk`, `vector`, `fake`, `index` and migration 0012.

*Amended 2026-11-09 (Slice 2.4):* Qwen3-Embedding is stored at **512
dimensions**. Its vectors may be cut short (Matryoshka), and on the vague
evaluation set 512 loses nothing against 1,024 (hybrid hit@5 0.950 against
0.925, on oMLX). The bundled catalog's `storedDimensions` marks the models
this applies to. The service still returns its full length, and the
backend keeps the first 512 numbers, normalised again. This halves the
f16 vectors and the index file. A library indexed at 1,024 builds a new
512 index while the old one serves (ADR 0014's model change).


## Amendment: related documents (Phase 2, Slice 2.5, 2026-11-10)

Related documents (phase2-spec §7.5) are the first reader of the stored
vectors that sends nothing to a model. They compare only what the catalog
and the index already hold (`services/documents/related.rs`).

**The document's vector** is the mean of up to 64 of its chunk vectors,
read from `embeddings` (not from the index), re-normalised:

- all of them for a document of 64 chunks or fewer;
- otherwise the first 32, where a book says most about itself, and 32 more
  spread evenly over the rest.

**The search:**

- The 60 nearest chunks, with `filtered_search` leaving out the document's
  own chunks through the chunk → document map, so they can't take the
  places.
- Chunks under the model's similarity floor are dropped (0.25, as search),
  and the rest grouped by document, keeping the best chunk.
- When that leaves fewer than ten documents, one search at 200. One long
  book's chunks can otherwise crowd the 60 nearest; the spec's k = 60 is
  widened only then.

**Never related:**

- **The document itself and its copies.** Copies are found by content
  hash, so they are the same document.
- **Documents with the same text in other bytes:** a re-saved PDF, or a
  repackaged EPUB. A candidate is left out when 95% of the larger
  document's distinct chunk texts are in both. A chapter published on its
  own, or a book containing it, stays related.
- **Documents with no available file.**

**Statuses, never errors.** `RelatedPage.status` reports:

- `off`: no embedding role, and the panel shows nothing;
- `not_indexed`: no done row for the serving model;
- `no_text`: the document's row is `empty`;
- `loading`: the index is opening in the background, and the panel asks
  again;
- `failed`: the index couldn't be searched;
- `ready`: the list was found, possibly empty.

**The cache:**

- It holds the candidates of 64 documents, keyed by model, index
  generation and index size. A save changes the generation; an add changes
  the size.
- Files are looked up afresh each time, so a file gone missing drops out
  at once.
- Measured, the cache saves little: at 1,000 documents a cold answer takes
  1.8 ms p50, and looking up the ten files is most of it.
