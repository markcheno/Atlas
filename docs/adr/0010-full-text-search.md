# ADR 0010: Full-text search

- **Status:** Accepted (Phase 1, Slice 1.4)
- **Date:** 2026-11-09
- **Context documents:** [Phase 1 spec](../phase1-spec.md) §6.1, §7.4, §10,
  §13; [ADR 0009](0009-jobs-and-extraction.md) (documents and passages);
  [ADR 0006](0006-search-keys-and-safe-opening.md) (Phase 0 name search)

## Context

Slice 1.3 stores each readable document's text as passages (one per PDF
page, or per EPUB chapter piece). Phase 1's main promise is finding a
passage by what it says. Search must stay fast while text is still being
read (p95 under about 250 ms on 30,000 files), must never let what a user
types become query syntax, and must find paraphrases, not only exact words:
the committed evaluation (40 questions, a third of them paraphrases) needs
a relevant document in the top 5 for at least 80%.

## Decision

**The text lives in the catalog, indexed by SQLite FTS5.** One source of
truth, transactional with the documents and files (a document and its
index entries commit together), and FTS5's `snippet()` needs the text in
the database. `content/extracted/` stays reserved. Migration 0009 adds an
external-content FTS5 table over `passages` with the three triggers from
the FTS5 documentation, then `'rebuild'`s it once, so text read by Slice 1.3
is indexed without reading any file again. After each reading pass, the
index is merged in bounded steps.

**Tokenizer: `unicode61 remove_diacritics 2`, `detail='full'`.** Case and
accents fold (`CAFE CREME` finds "café crème"), as the Phase 0 name search
does; full detail keeps phrase queries exact. Limitation: a run of Chinese
or Japanese characters is one token, so such text is found only by whole
runs. A trigram index is the Phase 4 option; the evaluation corpus's CJK
book proves it reads, and no question targets it.

**What the user types is data.** The query builder quotes every word and
phrase for FTS5 (inner quotes doubled), so `*`, `NEAR(`, parentheses,
`AND`/`OR`/`NOT`, `^`, `:` and quotes are searched as words and can never
cause a syntax error. The one operator a user can reach is a trailing `*`
on a word (a prefix). Punctuation-only input is an empty result, not an
error.

**All words, then any words** (product-owner decision). A passage must
contain every word. When fewer documents than one page match all words,
the any-words form (common words dropped, the rest ORed; phrases and
prefixes kept) fills the page after them, and those results say so
(`allWords: false`). The evaluation shows why: every one of the 13
paraphrase questions was answered by the fallback; with all words only,
the score would have been 27/40 (0.675), below the bar.

**Results are documents.** The best 500 passages by `bm25` are fetched,
then grouped by document in Rust (best passage kept), so a common word
over a large library never groups every match. Folder, type and missing
filters apply in SQL to the document's files. A file-name match (Phase 0's
folded `search_key`) marks a text match `why: both` and strengthens its
score by a quarter; files matched only by name (including ones not read
yet) follow the text matches. Each result shows one file (an available copy
inside the filters first) and how many other copies exist.

**Snippets are spans.** FTS5 `snippet()` (about 24 tokens) marks hits with
two private-use characters, U+E000 and U+E001; Rust turns them into
`SnippetSpan { text, hit }` lists, and the UI never receives markup.
Extraction strips those two characters from stored text, so a document
can't forge a highlight.
The Search view (Slice 1.5) renders each span as plain text, a hit inside
`<mark>`, and never uses `{@html}`; a Vitest case checks that HTML inside
a snippet stays text.
Typographic double quotes (`“ ” „ « »`) quote phrases like `"`, since
macOS smart quotes turn a typed `"` into `“` (Slice 1.5).

**Reads never wait for writes.** Search runs on the read-only connection
pool, raised from 2 to 4 connections so a slow search never starves the
status poll.

## Measurements (M5 Pro, 2026-11-09)

- **Evaluation:** 40/40 questions with a relevant document at rank 1
  (`tests/search_eval.rs`), with the questions as committed before search
  was built and no tuning. The corpus has only 17 searchable documents, so
  this is a floor check, not a quality claim; `just eval` runs the same
  check on a real library.
- **30k text fixture** (30,006 files, 293,070 passages, 622 million
  characters): `search_documents` p95 152 ms while text is being read, 175
  ms idle; the slowest kind, a prefix, 195 ms. The index is 248 MB (40% of
  the text); a full rebuild takes 7.5 s. See
  [benchmarks](../benchmarks.md#search-slice-14).

## Consequences

- The catalog grows by the text and its index: 1.4 GB for 622 MB of text
  (2.3×) as built in Slice 1.4, above the provisional 1.5× (phase1-spec
  §13). **Slice 1.7 gives new catalogs 16 KiB pages** (product-owner
  decision: new catalogs only). Passages run 2–4 KB, so in 4 KiB pages
  almost every passage had a page to itself, a third of it empty. On the
  30k text fixture a new catalog is now 963 MiB for 594 MiB of stored
  text, **1.62×**: the text's pages are 1.15× the text, the index 0.40×
  and everything else 0.08× ([benchmarks](../benchmarks.md#text-extraction-slice-17)).
  Reading is within 2–3% of the old speed (1k fixture, repeated runs). Catalogs created before Slice 1.7
  keep 4 KiB pages, since the page size can only change by rewriting the
  whole file; converting them is left to Phase 4's maintenance tools.
  The product owner accepted 1.62× as Phase 1's result at the close-out
  (Slice 1.8). The rest of the gap to 1.5× is the index, which phrase search needs
  (`detail='column'` would lose exact phrases). Compressing passages would
  save more, but this index reads the text from `passages` for snippets,
  deletes and rebuilds, so it would need a contentless index and snippets
  built by Atlas; not worth it for 0.12×.
- Opening a library made by Slice 1.3 rebuilds the index once, about 7.5 s
  per 600 MB of text on the M5 Pro, during the upgrade.
- The fallback can return looser results for long queries; the Search
  view (Slice 1.5) labels them "Some of your words".
- Storage accounting (Slice 1.6) measures the text and the index with
  SQLite's `dbstat` (compiled into the bundled SQLite) on a read-only
  connection of its own: exact, 9 ms on the 1k text fixture and 0.3 s at
  30k. The spec asked for `dbstat`-free queries; `dbstat` was available
  after all. It shows where the 2.3× growth comes from: at 30k the
  `passages` table takes 1,057 MiB for 594 MiB of text, the index 237 MiB
  ([benchmarks](../benchmarks.md#duplicates-and-storage-slice-16)).
- Semantic search (embeddings, ADR 0008's vector engine) arrives in a later
  phase and will rank alongside, not replace, this index.

## Amendment: hybrid search (Phase 2, Slice 2.4, 2026-11-09)

Search now uses meaning as well as words (phase2-spec §7.4). Phase 1's
ranking above is unchanged and runs first; `search::search` is still
exactly it, and `search::search_with` adds meaning.

**The meaning pass** (`services/search/hybrid.rs`):

- **Which model:** the query is embedded with the model search serves
  (`models::serving`), with the model's query prefix from the bundled
  catalog (Qwen3-Embedding:
  `Instruct: Given a search query, retrieve passages that answer it\nQuery: `).
  The embedding runs on its own thread while the text pass runs. Search
  waits for it at most 1.5 s; a slower request finishes in the background
  and fills a cache of 256 queries per model.
- **Nearest chunks:** the 200 nearest chunks in the index, with
  `filtered_search` when a folder or type is chosen. A dense chunk →
  document map (`embedding/chunk_map.rs`) answers which chunks a filter
  allows. It is topped up as chunks are added and never goes stale, since
  chunk ids are never reused, and the allowed documents come from
  `file_instances` per query.
- **Kept per document:** the best chunk, if its cosine similarity clears
  the model's floor (0.25 for Qwen3-Embedding, from the evaluation). At
  most 20 documents found by meaning alone take part, so nonsense still
  finds nothing.

**Fusion:**

- **Reciprocal rank fusion**, k = 60, over the text list's document order
  and the meaning list's, **in two tiers**.
  - Tier 1: documents containing every word come first, fused with
    meaning among themselves.
  - Tier 2: documents with some of the words, and those found by meaning
    alone, fused together.
  - A single RRF over both lists let an all-words match fall behind
    partial matches that also matched by meaning (q04 of Phase 1's set,
    with the fake backend), so the tier keeps the strongest evidence a
    search has on top.
- **Name matches** still strengthen a result by ×1.25.
- **Without a meaning list** (meaning search off, failed, or finding
  nothing), the order is exactly Phase 1's.
- **Reasons:** `Semantic` (similar meaning only) and `TextSemantic` (in the
  text and similar meaning). A meaning match never says "Some of your
  words".
- **Snippets:** a meaning-only hit's snippet is its chunk, trimmed to about
  240 characters at word boundaries, with query words marked by folded
  match.

**Never an error.** `SearchPage.semantic` says what happened:

- `Off`: no embedding model;
- `Ready { coveragePercent }`;
- `Unavailable { not_permitted | unreachable | timeout | loading | failed }`.

The page is otherwise exactly Phase 1's. An index that isn't open yet is
opened in the background, and the Search screen searches again when the
embedding status next changes.

**Off the workspace lock.** The search command takes the library's handles
(the reader pool, root status, vector registry) and runs without holding
the workspace, so a query's embedding never blocks other commands.

**Measured.**

- **On oMLX (Qwen3-Embedding-0.6B):** hit@5 on the 40 vague questions is
  0.950 hybrid (0.875 for text alone), and Phase 1's set stays 40/40
  ([benchmarks](../benchmarks.md#meaning-search-slice-24)).
- **With the fake backend on every OS:** Phase 1's set is 39/40. The fake's
  word-overlap "meaning" misses q40's long chunk, so partial matches fuse
  above it.
- **P2-18, search while the embedding job runs:** see the benchmarks.

