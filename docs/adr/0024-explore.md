# ADR 0024: Explore, a map of the library by meaning

- **Status:** Accepted (Phase 4, Slice 4.6.1)
- **Date:** 2026-11-15
- **Context documents:** [Phase 4 spec](../phase4-spec.md) revision notes
  and §12 (P4-31 to P4-34); [the Explore spike](../spike-explore.md);
  [ADR 0004](0004-resource-policy.md) (the resource policy);
  [ADR 0013](0013-embeddings-durable-index-disposable.md) (vectors in the
  catalog); [ADR 0023](0023-one-book-two-formats.md) (one book, two formats)

## Context

The Explore spike (Slice 4.1.2) asked whether a library can be laid out by
meaning, as named areas on a map. On the product owner's 5,088 books the
answer was go:

- all 16 areas belonged together and were recognisable from their words;
- the map kept 97.8% of books' neighbours;
- the whole run took 11.8 s.

The owner asked for it in the app next, with these choices:

- its own view;
- recomputed by itself but kept stable;
- areas they can rename;
- a Library filter by area;
- search results lit up on the map.

The spike read each book's vector as up to 64 single-row reads (7.3 s of
its 11.8 s), drew from scratch every time, and wrote one HTML page. None of
that suits an app that keeps a map current while the library grows.

## Decision

- **A vector per book is stored**, in `book_vectors` (migration 0018). It
  is the unit mean of the same sample of chunks Related documents
  averages.
  - It is written in the transaction that writes the book's embedding, so
    it never lags.
  - A library embedded before 4.6.1 has its vectors filled in, 200 books
    a writer call, the first time the map is drawn. That reads the
    catalog only; nothing goes to the AI service.
  - A copy kept out of a book in two formats is never on the map.
- **The map is derived data in the catalog:**
  - `explore_maps`: one row per model, with what it was drawn from (the
    embeddings' sequence number, the pairs' stamp) and a generation;
  - `explore_areas`: areas and sub-areas, each with a stable number, a
    colour, its words, its central books and the owner's name;
  - `explore_points`: where each book sits.

  All of it can be dropped and redrawn, except the owner's names, which
  follow their areas.
- **A watch thread, not a job,** keeps it current: one thread per open
  library, like the folder check.
  - **Why a thread:** a job row belongs to one folder, and a map is of the
    whole library.
  - **When it looks:** every minute. It draws when the map is due: no map
    yet with at least 30 books, or books new, changed or gone amounting to
    5% of the map's books (at least 10).
  - **When it may draw:** only while the policy says Run, normally or at
    full speed, not when work is slowed or paused.
  - **A redraw the owner asks for** runs even when work is slowed. It
    waits while paused and says why.
  - **Reading and writing:** reads go through the read-only connections;
    the result is written in one transaction.
  - **Stopping:** the layout stops at its next pass when the library
    closes.
- **Stable redraws:**
  - **Areas start from the old ones:** each old area's books still here
    make a starting centre, and every book goes to its nearest. Areas are
    split further only when the library has grown past the old count.
  - **A new area continues an old one** when they share at least 30% of
    the larger of the two. It keeps the old number, colour and name.
  - **Books on the old map stay exactly where they were;** new books start
    among their neighbours and settle.
  - **Redraw from scratch** draws as the spike did, but still keeps names
    and numbers where areas survive.
- **The number of areas follows the library:** the square root of half
  the books, from 2 to 16. A small library isn't cut into areas of three
  books, and the owner's library has 16, as in the spike.
- **Colours** are eight hues of the reference chart palette, chosen in
  Rust from how areas touch on the map. Areas side by side never share a
  hue or take a pair too close to tell apart. An area keeps its hue unless
  that would clash.
- **What the app doesn't compute:** the spike's measures (trustworthiness,
  silhouette, the second variant). They judged the method; they stay in
  the harness (`just spike-explore`).

## Consequences

- **The cost of a draw:** a first map of 5,000 books takes about 2 s and
  a redraw 0.4 s; at 30,000, 14 s and 10 s, mostly finding neighbours.
  Both run on half the cores, in the background
  ([benchmarks](../benchmarks.md#explore-slice-461)).
- **The window reads the whole map at once:** 0.6 MB at 5,000 books and
  3.5 MB at 30,000, once a generation.
- **An old arrangement persists,** since books never move on a redraw.
  When the library's shape has changed a lot, Redraw from scratch
  rearranges it.
- **A book embedded after the map** isn't on it until the next redraw.
  The status line counts them.
- **Names from the chat model and "what the library is made of"** remain
  candidates ([the spike](../spike-explore.md#the-answer-is-go)).
