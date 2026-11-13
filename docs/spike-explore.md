# Slice 4.1.2 spike: a map of the library by meaning

**Status:** harness built and measured on synthetic data; the owner's run
on the real library is to be recorded. The rule below was written before
any real library was measured.

The product owner asked (2026-11-12) for visual ways to browse: a semantic
explorer, with books clustered by meaning into named **areas**, shown as a
map and as a list. This spike answers whether that is worth building, on
the only evidence that can: the owner's real collection, looked at by its
owner.

It changes nothing in the app. There is no migration, no command and no
screen. The code is in `src-tauri/src/services/explore/` and the harness
in `src-tauri/tests/explore.rs`.

## The question

1. Do the areas hold books that belong together?
2. Can an area be recognised from a name made of its books' own words,
   with no AI?
3. Does a flat map keep near books near?
4. Is it quick enough to do in the background?

## What the spike does

Everything is computed from vectors the catalog already holds. Nothing is
sent to a model or anywhere else.

- **One vector per book** (`books.rs`): the unit mean of the same sample
  of up to 64 chunk vectors that Related documents averages
  (`related::sample_indices`), for one embedding model. Books without
  vectors (not read, not yet indexed, refused by the model) are counted
  and left out.
- **Areas** (`cluster.rs`): spherical k-means built by bisecting, to 16
  areas, then a few rounds of every book to its nearest centre. Each area
  is split once more into up to 4 sub-areas. Seeded: the same seed gives
  the same areas.
- **Neighbours** (`knn.rs`): each book's 15 nearest, exactly, on threads.
- **The map** (`layout.rs`): a start from the two widest directions (PCA),
  then UMAP's pulls along neighbour edges and pushes from random books.
  One thread and a seeded generator.
- **Measures** (`quality.rs`): trustworthiness and neighbour preservation
  of the map, whether areas show as regions on it (area agreement), a
  silhouette of the areas, and pairs of books so alike they are probably
  one text twice.
- **Names** (`names.rs`): for each area, the words that many of its most
  central books use and other areas' books mostly don't, plus the titles
  of its three most central books. A word counts once per book, from the
  title and the three chunks nearest the area's centre, and must be in at
  least two books.

### Why hand-written

`CONTRIBUTING.md` prefers maintained libraries. Here the methods are
textbook and short (about 2,100 lines with their tests). The alternative was
several new crates to license, build on three OSes and then remove if the
answer is no. No dependency was added. If the spike says go, the choice is
looked at again for the real feature.

### Why in the app's crate

Earlier spikes live in `spikes/`, outside the app. This one reads the
catalog through the app's own code (the schema check, the vector
encoding, the sampling Related documents uses, the title rules), and on
"go" its functions become the feature's. Nothing in the app calls the
module. If the spike says stop, the module is removed.

## The harness

```sh
just spike-explore "<the library's folder>"          # the page goes to src-tauri/target/tmp/explore
just spike-explore "<the library's folder>" ~/Desktop/explore
```

- **Read-only** (`catalog::reader::open_offline`). With Atlas quit, the
  catalog is opened as immutable and nothing in the library's folder is
  created or touched. Beside a running Atlas, the read marks itself in the
  catalog's `-shm` file, which is already there, and nothing else changes.
  The catalog is never migrated: a catalog of another schema than the
  build's is refused.
- **The terminal shows numbers only**: counts, measures and timings.
  Models are listed by position, never by name.
- **The page** (`atlas-explore.html`) holds titles and area names, which
  are words from the books. So the harness refuses to write it inside the
  library, or inside the repository anywhere but `src-tauri/target/`
  (ignored by git; `just clean` removes it). It is one file with no
  address in it and a policy that allows none.
- **On the page:** the map (drag, scroll to zoom, hover for a title), the
  areas with their names, central titles, sub-areas and every book, two
  ticks per area, a running tally, and a box of numbers to share that
  holds no title, word or name.
- **Sixteen areas are too many to tell apart by colour.** The map draws
  every book alike, numbers each area at its centre, and lights up one
  area at a time in a single colour.
- **Options** (environment variables):
  - `ATLAS_EXPLORE_SAMPLING=all`: every chunk, not the sample;
  - `ATLAS_EXPLORE_CENTRED=1`: take out what every book shares first;
  - `ATLAS_EXPLORE_AREAS=<n>`;
  - `ATLAS_EXPLORE_MODEL=<position>`: by default the model with the most
    documents done, which matters after a change of model.

## The rule

**Precondition.** At least 90% of the documents with text have a vector.
Below that the page says the run is preliminary, and it is repeated.

**Fixed before measuring.** The sample of 64, 16 areas, 15 neighbours,
seed 1.

**Go if all four hold:**

1. **Areas.** The owner ticks at least 12 of the 16 as "these books belong
   together", and the ticked areas hold at least 70% of the books, so one
   large junk area can't pass on count alone.
2. **The map.** Trustworthiness at 15 neighbours is at least 0.85 and area
   agreement at 10 is at least 0.70. PCA alone is recorded beside it.
3. **Time.** The whole run takes at most 120 s on the owner's machine, and
   at most 60 s after loading.
4. **Names.** The owner ticks at least 60% of the areas as "I'd recognise
   the area from these words".

**Otherwise:**

- **Names fail, areas pass:** go, with names from the chat model as part
  of the build. The local words and central titles stay as the label when
  AI is off.
- **The map fails, areas pass:** build the list of areas and not the map.
- **Time fails:** not a stop. A stored vector per book and a background
  job come first.
- **Areas fail:** one more run with every chunk
  (`ATLAS_EXPLORE_SAMPLING=all`), then one with what every book shares
  taken out (`ATLAS_EXPLORE_CENTRED=1`), each ticked afresh. Every run
  prints the silhouette of both the plain and the centred vectors, as a
  hint of which will do better. If none passes, stop: the module is
  removed.

The thresholds are provisional. They were set with no real library to
check them against, and the owner's first run may show one is wrong; a
change is recorded here with its reason before a second run is judged.

**"What the library is made of"** (blocks by folder and format, coloured
by state) needs no vectors. It is specified whatever this spike finds.

## Results

### Synthetic books (2026-11-12, M5 Pro, 18 threads)

`just bench-explore`: books drawn around 40 planted centres, 512
dimensions, grouped into 16 areas. There is no library, so nothing is
loaded or named.

| Stage | 5,000 books | 30,000 books |
| --- | --- | --- |
| Neighbours (exact, 15) | 0.14 s | 6.8 s |
| Areas and sub-areas | 0.21 s | 1.6 s |
| The map | 2.2 s | 5.9 s |
| Measures | 1.7 s | 12.3 s |
| **All, after loading** | **4.3 s** | **26.6 s** |
| Vectors in memory | 9.8 MiB | 58.6 MiB |
| Trustworthiness at 15 (PCA alone) | 0.99 (0.69) | 0.99 (0.71) |
| Area agreement at 10 | 1.00 | 1.00 |

- **The measures are the slowest part at 30,000**, and the feature
  wouldn't compute them: they exist to judge the spike.
- **Planted centres are far easier than books.** These numbers say what
  the method costs and that it works when the answer is clear. They say
  nothing about a real library.
- **Neighbour preservation is low here by construction** (0.16 and 0.03):
  inside one planted group every book is as near as every other, so there
  are no true nearest neighbours to keep. On real vectors it is recorded
  with the rest.

### The owner's library

To be recorded: coverage, books, areas' sizes and cohesion, the map's
measures, timings, and the ticks.

## If the answer is go

Candidate slices, to be specified then (not Phase 4 scope until decided):

1. **A vector per book, and an areas job.** A table of book vectors for
   the serving model, filled as books are embedded; areas, names and the
   map computed in the background under the governor, kept as derived
   data. New books are placed among their neighbours; a full re-layout
   starts from the old map so it doesn't reshuffle.
2. **Areas in the Library.** The list: areas, sub-areas and their books,
   which is also the map's text equivalent.
3. **The map.** A new view on a canvas, with pan and zoom, opening the
   details panel.
4. **Names from the chat model**, optional, with the disclosure that
   titles are sent to it.
5. **What the library is made of.**

Not chosen for now: search results and trails on the map, a walk from one
book through its neighbours, and books that bridge two areas.
