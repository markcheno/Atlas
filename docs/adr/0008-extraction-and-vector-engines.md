# ADR 0008: Extraction libraries and vector engine

- **Status:** Proposed. The criteria below were committed on 2026-11-08,
  before any candidate was measured. The decision section is filled in from
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

*Pending measurements.*

## Consequences

*Pending measurements.*
