# ADR 0023: One book in two formats

- **Status:** Accepted (Phase 4, Slice 4.5.1)
- **Date:** 2026-11-13
- **Context documents:** [Phase 4 spec](../phase4-spec.md) revision notes
  and §12 (P4-27 to P4-30); [ADR 0009](0009-jobs-and-extraction.md) (a
  document is its bytes); [ADR 0013](0013-embeddings-durable-index-disposable.md)
  and [ADR 0014](0014-the-embedding-job.md) (meaning)

## Context

The product owner's library holds many books twice, as a PDF and an EPUB,
almost always in one folder with one name apart from the extension.

A document is the BLAKE3 hash of a file's bytes, so the two copies were
unrelated documents, and every stage paid for both:

- **Meaning indexing:** about 2× the AI service's work and 2× the vector
  index. Chunks are reused only when their text is byte-identical, and a
  PDF's pages and an EPUB's chapters cut the text differently.
- **Search:** two results side by side, each with "0 other places".
- **Related:** the other copy was likely the top hit. Its 95% same-chunk
  rule doesn't fire across formats.
- **Ask Atlas:** the pair took two of its eight sources.
- **The Explore map:** two points almost on top of each other.

The owner's library was about 24 hours into indexing when this was
decided. So the change must not index anything again, and nothing indexed
may be lost.

## Decision

**Two tables, one for each level** (migration 0017):

- **`format_pairs`** holds the evidence from names, about files. It pairs a
  PDF and an EPUB that are available, in one folder of one source root, with
  names that fold to the same text apart from the extension. Folding is the
  Library's own: case, accents and macOS's decomposed names don't matter. A
  name with anything but exactly one PDF and one EPUB pairs nothing. This
  table is rebuilt from the names and holds no decision.
- **`book_pairs`** holds the verdict, about documents. It records whether
  two documents are one book, on what basis (`text`, `name` or `owner`),
  which format is indexed and shown, and, while the book is active, the
  document kept out. A book is active while its verdict is "same" and some
  folder holds a file of each, side by side and available. A document is in
  one active book at most.

**The text check** (`formats/compare.rs`, deterministic, no AI):

- **Preparation:** both texts are normalised. PDFium's hyphenation mark and
  soft hyphens are joined, a hyphen at a line's end before a lowercase
  letter is joined, ligatures and accents are folded, and the text is
  lowercased. CJK characters count as a word each.
- **Sketch:** every run of 5 words is hashed with BLAKE3, and the hashes
  divisible by 8 are kept.
- **Same book:** at least half of the smaller sketch is found in the larger
  (containment, because a PDF adds running heads and page numbers), and the
  lengths are within a factor of two (so a sample chapter isn't its book).
- **Too short to sample** (under about 500 words): the lengths and what
  little was sampled decide.
- **No text on one side** (a scan that needs OCR, a locked or unreadable
  file): the name alone pairs them, and the side with text is the one used.
- **Speed:** a pair of books of about 400 KiB of text takes 26 ms at p50
  and 31 ms at most on the M5 Pro
  ([benchmarks](../benchmarks.md#one-book-two-formats-slice-451)).

**Which copy is indexed and shown:**

1. The owner's choice.
2. A copy already indexed for a model in use. This is what guarantees
   nothing is indexed again; if both are, the EPUB.
3. The EPUB, if it has text.
4. The PDF.

Once chosen, the format is kept while it has text.

**When it runs:**

- **A reading job's end:** the folder's names, the text check for pairs not
  yet checked (or re-read since), then the active books. This happens
  before the meaning job is queued, a few pairs per catalog call, and stops
  when asked.
- **A meaning job's start**, on a folder not yet paired by this build.
- **A scan's finish:** names only. A copy whose partner went away is shown
  again and queued for its meaning.
- **A library opening:** a folder not yet paired (`formats_version`) gets a
  reading job, which finds nothing to read and pairs as it ends. A folder
  added from now on starts as current, since its first read pairs it.
- **The owner's choice** (`set_same_book`) is kept over every later check.

**What each part does with a book:**

- **The meaning queue** skips the copy kept out. Counts give it its book's
  status, so it never shows as waiting. Coverage, the estimate and a model
  rebuild's coverage check leave it out; without that last one, every
  rebuild would fail its check.
- **Search** groups matches by book, keeping the best passage from either
  copy. Full-text matches in the copy kept out still count, so a scanned PDF
  is findable through its EPUB, and a PDF match keeps its page. Meaning
  matches come only from the copy shown: the hidden copy's vectors are
  excluded by a filtered search. A result carries `otherFormat`. With a file
  type chosen, each format stands alone.
- **Ask Atlas:** a book takes one source, and a meaning excerpt cites the
  file it came from.
- **Related and rediscovery** resolve either copy to the copy shown, and
  never offer the other copy or any copy kept out. Related's cache key
  includes the active books.
- **The Library** lists a book once (as "EPUB · PDF") unless a type or a
  text status is chosen.
- **The detail panel** says why the two are one book (or not) and offers
  **Not the same book** or **Same book**.
- **The Explore spike** draws one point a book.

**Unchanged:** Duplicates (identical bytes only), the knowledge store
(document ids and hashes don't change), and every passage and vector.

## Consequences

- **The owner's library pairs once, at the first open of the 4.5.1 build.**
  Copies already indexed stay the copy used, nothing is sent to be indexed
  again, and the PDFs of pairs not yet indexed never will be.
- **A wrong pair costs little.** Only text-confirmed pairs (or a scan beside
  its EPUB) hide a copy, the hidden copy's text stays searchable, and the
  owner can undo it in one click.
- **Writer time:** about 26 ms a pair, in calls of four pairs.
- **Search with any copy kept out** always uses the filtered vector search.
- **Left out:**
  - pairs across folders or roots, or with formats other than PDF and EPUB;
  - editions and translations;
  - Show the page for an EPUB match;
  - deleting the vectors of copies kept out (un-pairing is instant because
    they stay).
