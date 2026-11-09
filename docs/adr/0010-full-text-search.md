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
  (2.3×), above the provisional 1.5× (phase1-spec §13). Revisited with the
  extraction benchmark in Slice 1.7 (options: `detail='column'` loses exact
  phrases; compressing passages loses nothing but CPU).
- Opening a library made by Slice 1.3 rebuilds the index once, about 7.5 s
  per 600 MB of text on the M5 Pro, during the upgrade.
- The fallback can return looser results for long queries; the Search
  view (Slice 1.5) labels them "Some of your words".
- Semantic search (embeddings, ADR 0008's vector engine) arrives in a later
  phase and will rank alongside, not replace, this index.
