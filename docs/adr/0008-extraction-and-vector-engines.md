# ADR 0008: Extraction libraries and vector engine

- **Status:** Accepted. The criteria were committed on 2026-11-08 (commit
  b3e6a64), before any candidate was measured; the decision follows from
  the measurements.
- **Date:** 2026-11-08
- **Context documents:** [Phase 0 spec](../phase0-spec.md) §11 (spike
  corpus), §15 (Slice 0.9) and done-criterion 10;
  [product spec](../product-spec.md) technology table; results in
  [Slice 0.9 spike](../spike-0.9.md)

## Context

Phase 1 extracts the text of PDFs and EPUBs into the catalog for full-text
search, as background work that respects the resource policy (ADR 0004).
Phase 2 embeds that text and searches it semantically at millions of
chunks. Both depend on libraries that are expensive to change later: they
decide what ships in the installer, how the build is set up on each OS,
whether a crash can take the app down, and how much disk an index takes.

The spike measures candidates on a real, redistributable corpus and on
synthetic vectors. It is not wired into the app. The harness lives in
[`spikes/`](../../spikes/README.md).

## Candidates

**Extraction.**

| Candidate | Role | License |
| --- | --- | --- |
| `pdfium-render` + PDFium (bblanchon/pdfium-binaries, shared library) | Native PDF baseline | MIT/Apache; PDFium BSD-3 with third-party notices |
| `pdf-extract` | Pure-Rust reference: the cost of shipping no native library | MIT |
| Docling, frozen with PyInstaller (CPU PyTorch) | Bundled sidecar; measures the real cost of "no Python installation" | MIT; models Apache-2.0 |
| `rbook` + `html2text` | EPUB | Apache-2.0; MIT |

**Vectors.** `sqlite-vec` 0.1.9 (brute force in SQLite, which Atlas
already uses), `usearch` 2.x (HNSW), `lancedb` (IVF-PQ / IVF-HNSW-SQ on
Lance files). The `sqlite-vec` 0.1.10 alpha's DiskANN index is measured for
information only; an alpha can't be adopted.

**Excluded on license before measuring:** MuPDF and PyMuPDF (AGPL-3.0); the
`epub` crate (GPL-3.0).

## Decision criteria

Fixed before measuring. A candidate that fails a hard gate is out
regardless of its other numbers.

**Hard gates (every area).**

- A permissive license for everything that would ship, including
  transitive dependencies and models.
- Builds on macOS arm64, Windows x64 and Linux x64 with only downloadable
  tools (no system-wide installs).
- A malformed input never crashes or hangs the calling process.

**PDF.**

- **Baseline (pdfium).**
  - Born-digital text: word F1 ≥ 0.99.
  - Ligatures: 100% correct where the font has a ToUnicode map.
  - Two-column reading order ≥ 0.95.
  - Scanned pages: 100% flagged "needs OCR", with ≤ 2% false positives.
  - At least 20 pages/s on one core of the M5 Pro.
  - If the child process crashes (segfault) on any file, the finding is
    that extraction must run in a helper process.
- **Docling becomes the default bundled extractor only if all of these
  hold:**
  - it beats pdfium by ≥ 0.05 in reading order on two-column and arXiv
    documents;
  - it adds ≤ 250 MB to the compressed installer;
  - cold start ≤ 5 s;
  - warm extraction ≤ 1 s per page on CPU.

  Otherwise it becomes, at most, an optional component, and this ADR states
  what would reopen the question.

**EPUB.** Word F1 ≥ 0.98 and spine (chapter) order ≥ 0.98 against Project
Gutenberg's own plain-text edition; no panics.

**Vectors** (1,000,000 × 1,024-dimension vectors, on the M5 Pro).

- **Unfiltered search:** recall@10 ≥ 0.95, with single-thread p95 ≤ 100 ms.
- **Filtered to one folder (about 5% of vectors):** recall@10 ≥ 0.90,
  p95 ≤ 150 ms.
- **Opening the index until the first answer:** ≤ 2 s.
- **On disk:** ≤ 1.5× the raw vector size, or the quantization is stated.
- **Deleting vectors** works without a full rebuild.
- **Build cost:** stripped binary grows by ≤ 15 MB, and a clean release
  build by ≤ 5 minutes.
- **Comparison method:** engines are compared at matched recall (sweeping
  their search parameters), never at defaults alone.
- **Tie-break:** fewer storage formats. `sqlite-vec` reuses the catalog's
  SQLite and its transactions.

If nothing passes, the result is a narrowed shortlist and the exact
question Phase 1 (extraction) or Phase 2 (vectors) must answer.

## Decision

Measurements: [Slice 0.9 spike](../spike-0.9.md). Unless a CI column is
named, figures are from the M5 Pro.

### PDF: PDFium through `pdfium-render`, in a helper process

| Criterion | Target | pdfium | Pass |
| --- | --- | --- | --- |
| License | permissive | BSD-3-Clause + notices; crate MIT/Apache | ✅ |
| Builds on 3 OSes | downloadable tools only | @@PDFIUM_CI@@ | @@PDFIUM_CI_PASS@@ |
| Malformed input | no crash or hang | 0 of 10 malformed PDFs | ✅ |
| Born-digital F1 | ≥ 0.99 | 0.990 (lowest 0.961, footnote markers and page numbers) | ✅ |
| Ligatures with ToUnicode | 100% | 100% (0 left) | ✅ |
| Two-column order | ≥ 0.95 | 1.000 | ✅ |
| Scans flagged | 100%, ≤ 2% false | 3/3 (with Atlas's check), 0/13 false | ✅ |
| Throughput | ≥ 20 pages/s, one core | 1,294 pages/s | ✅ |

PDFium never crashed. Even so, **extraction runs in a helper process**: a
second mode of the Atlas executable, started per batch. The reasons:

- PDFium is C++ reading untrusted files.
- It isn't thread-safe; pdfium-render serializes every call.
- A worker can be killed on a timeout or a memory cap, which the resource
  policy (ADR 0004) needs anyway.

Starting a worker costs about 3 ms.

**Docling is not bundled.** It fails two of its four conditions:

- it adds 982 MB compressed (limit 250 MB);
- it doesn't read papers in a better order than pdfium (0.868 vs 0.901
  against arXiv's HTML; the threshold was +0.05).

Its cold start (3.1 s; 22.6 s the first time after install) and warm speed
(0.16–0.46 s/page on CPU) are within limits. It also uses 3.8 GB of memory
and needed three PyInstaller fixes to run at all. Reopen the question if
users need **table structure** or **OCR of scanned books**. Docling's
TableFormer and layout models are what pdfium lacks. At that point, offer
it as an optional download, or run its ONNX layout model from Rust,
instead of shipping Python.

**pdf-extract is not chosen.** It has no native library to ship, but it
leaves ligature characters in the text, missed 23% of one paper, and
rejected a PDF that pdfium recovered.

### EPUB: `rbook` + `html2text`

F1 0.996 and chapter order 1.000 against Project Gutenberg's own text
(threshold 0.98 for both), and no panics on five malformed EPUBs. Both are
pure Rust and permissive (Apache-2.0, MIT) and add 1.8 MiB.

### Vectors: `usearch` (HNSW, f16), with Atlas owning durability

| Criterion | Target | usearch f16 | LanceDB | sqlite-vec |
| --- | --- | --- | --- | --- |
| Unfiltered recall@10, p95 | ≥ 0.95 at ≤ 100 ms | ✅ 0.963 at 0.84 ms | ✅ IVF-PQ 1.000 at 24.5 ms; HNSW-SQ 0.953 at 0.86 ms | ❌ exact at 517 ms (f32, NEON); 0.954 at 157 ms (int8) |
| Filtered at 5% | ≥ 0.90 at ≤ 150 ms | ✅ 0.988 at 6.8 ms | ✅ 1.000 at 24 ms | ✅ 1.000 at 26 ms |
| Open → first answer | ≤ 2 s | ✅ 0.50 s | ✅ 0.05 s | ✅ 0.50 s |
| On disk | ≤ 1.5× raw | ✅ 0.54× (f16) | ✅ 1.02–1.30× | ✅ 1.01× (int8 0.26×) |
| Delete without rebuild | yes | ✅ tombstones | ✅ deletion files | ✅ rows |
| Stripped binary delta | ≤ 15 MB | ✅ +1.8 MB | ❌ +192.6 MB | ✅ +0.2 MB |
| Clean build delta | ≤ 5 min | ✅ +12 s | ⚠️ +4.4 min on the 18-core M5; @@LANCE_CI@@ | ✅ +0 s |
| License | permissive | ✅ Apache-2.0 | ✅ Apache-2.0 (one MPL-2.0 dependency) | ✅ MIT/Apache |

**usearch** meets every criterion. **LanceDB** meets the search criteria
but adds 13 times the allowed binary size (Arrow, DataFusion and Lance
come along). Its build stays under five minutes only on an 18-core machine.
It is also the heaviest to run (4–5 GB of its own memory at 1M) and needs
`protoc` to build. **sqlite-vec** is brute
force in its stable release, so whole-library search at a million vectors
is too slow. Its ANN alpha didn't produce usable results, and an alpha
can't be adopted anyway.

### Rules for whoever builds on this

- **Detect unreadable text.** Atlas flags a document for OCR when at least
  half its pages are nearly empty, or when at least 10% of the characters
  are control, private-use or replacement characters. Without this, a PDF
  without ToUnicode maps is indexed as garbage, silently, by pdfium and
  Docling alike.
- **Guard EPUB input.** Check `META-INF/encryption.xml` (DRM), and cap the
  uncompressed size of entries before reading them: a 64 MiB chapter cost
  rbook 336 MiB of memory.
- **Make the vector index derived and disposable.** usearch's `save`
  rewrites its file in place, and a kill mid-save left an index that no
  longer loads. So:
  - Embeddings are the durable data, kept with the catalog.
  - The usearch index is built from them, saved to a temporary file,
    flushed, then renamed over the old one.
  - An index that fails to load is rebuilt.
  - Index files are versioned per embedding model under `indexes/vectors/`
    (product spec: a new model builds a new index while the old one keeps
    serving).
- **Filter by folder** with `filtered_search` and a root lookup. At 0.5%
  selectivity it costs 54 ms (graph search slows as the filter narrows).
  For very small scopes, scoring the scope's own vectors directly is
  cheaper.

## Consequences

- **Phase 1 adds PDFium** as a shared library shipped beside the app on
  each OS (about 7 MiB unpacked), with its third-party notices in the app's
  license report.
  - The library comes from a pinned bblanchon/pdfium-binaries release with
    a checked hash, as `spikes/tools.lock.json` does now.
  - pdfium-render's `pdfium_NNNN` feature must match that release.
  - It has to be included in macOS signing.
- **Phase 1 adds the extraction helper process and its job type.** This is
  the second job kind, which triggers the planned migration of
  `scan_sessions` into a general jobs table (phase0-spec §6).
- **No Python in the installer.** OCR and table structure are open Phase 1
  questions. If needed, they come as an optional component, not by
  default.
- **Phase 2 adds usearch** (C++17, compiled by `cc`; about 2 MiB). The
  embeddings it indexes need a durable home of their own. The index can
  always be rebuilt, so its torn-file failure mode is a slow start, not
  data loss.
- **Revisit vectors** if:
  - real embeddings behave very differently from the synthetic set (Phase 2
    measures this first);
  - sqlite-vec ships a stable ANN index (one file, transactional with the
    catalog, would beat a separate index);
  - LanceDB's build cost drops.
- The harness stays in `spikes/` so these measurements can be repeated when
  any of the above changes.
