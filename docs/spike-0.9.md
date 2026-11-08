# Slice 0.9 spike: text extraction and vector engines

These are measurements, not guarantees (phase0-spec §13, §15). They inform
[ADR 0008](adr/0008-extraction-and-vector-engines.md), whose criteria
were written down before anything was measured. The harness is in
[`spikes/`](../spikes/README.md) and is not part of the app.

## Machines (2026-11-08)

| Label | Machine |
| --- | --- |
| **M5 Pro** | MacBook Pro (Mac17,9), Apple M5 Pro, 18 cores, 64 GB, macOS 27.0.1. Nothing else running |
| **CI** | GitHub-hosted runners: macOS arm64 (3-core M1 VM, 7 GB), Windows Server 2025 x64 (4 cores, 16 GB), Ubuntu 24.04 x64 (4 cores, 16 GB). Shared, noisy machines; use them to compare OSes and check that things build and run |

CI evidence: Spikes workflow run
[42505069661](https://github.com/markcheno/Atlas/actions/runs/42505069661),
all 12 jobs green (extraction, Docling sidecar, vectors at 100k, build cost;
each on macOS, Windows and Linux). Every job uploads its JSON and tables as
a `spike-*` artifact.

## The corpus

`spikes/extraction-bench/corpus/` holds 33 documents (9.4 MiB). Each file's
source, license and SHA-256 are in `manifest.json`; `just spike-corpus`
checks them.

| Kind | Documents | What the truth is |
| --- | --- | --- |
| EPUB | 5 Project Gutenberg EPUB3s, unmodified: Alice (prose), The Raven (poetry, illustrations), Candide (French), 道德經 (Chinese), The Prince (notes) | Gutenberg's own plain-text edition of the same book |
| Born-digital PDF | Typeset with typst from public-domain text: one column with a running header, two columns, a whole book (44 pages), and a page with ligatures, French accents, a table and footnotes; the same one-column file with an owner password only | The exact text that was typeset |
| Real PDF | 6 arXiv papers under CC BY 4.0 (checked on each abstract page) and 2 three-column Federal Register documents | arXiv's HTML rendering; the Federal Register's plain-text edition. These are references, not exact truth: the HTML has no running heads, places figures and footnotes differently, and Federal Register pages also carry the end of the neighbouring document |
| Text that can't be read | The one-column PDF with every font's ToUnicode map removed; a page rendered as an image, rotated and speckled; four pages of an 1894 Alice scanned by the University of California Libraries (Internet Archive) | Should be detected and sent to OCR |
| Malformed | Empty file, text named `.pdf`, truncated PDF, broken cross-reference table, a stream `/Length` far past the data, an object nested 200,000 arrays deep, a password-protected PDF; EPUBs without an OPF, of random bytes, with a `../../` path, with DRM (`encryption.xml`), and a 64 MiB chapter that compresses to 97 KiB | No crash, no hang; the right outcome where there is one (encrypted) |

## Extraction

**Metrics.** Text is normalized (NFKC, lower case, line-end hyphenation
joined; CJK characters count as single tokens).

- **F1** compares the multisets of words.
- **Recall** is the share of the truth's words found.
- **Reading order** is the share of the shared words that also come in the
  same order (a longest common subsequence). It is shown only when at least
  5% of the words were found.
- **Needs OCR**: a document is flagged when at least half its pages have
  fewer than 20 visible characters, or when at least 10% of its characters
  are control, private-use or replacement characters (glyph ids instead of
  text). This check is Atlas's, not the extractor's.

Every document is run once in its own process with a 30 s timeout, so a
crash or a hang is recorded instead of stopping the run. Throughput comes
from a second pass with all text documents in one process.

### Results on the M5 Pro

| Candidate | Born-digital F1 (lowest) | Two columns, order | Real PDFs, recall / order | EPUB F1 / order | Unreadable text flagged | Flagged by mistake | Ligature characters left | No crash or hang | Pages/s (44-page book) | Start ms | Peak memory | Size |
| --- | --- | ---: | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| **pdfium** | 0.990 (0.961) | 1.000 | 0.961 / 0.901 | – | 3/3 | 0/13 | 0 | 23/23 | 1,294 | 3 | 27 MiB | 5.8 MiB + 7.0 MiB library |
| pdf-extract | 0.992 (0.973) | 1.000 | 0.945 / 0.904 | – | 3/3 | 0/13 | 29 | 23/23 | 1,134 | 2 | 63 MiB | 4.1 MiB |
| Docling (frozen) | 0.992 (0.967) | 1.000 | 0.969 / 0.868 | 0.826 / 1.000 | 3/3 | 0/18 | 0 | 33/33 | 7 | 3,139 | 3.8 GB | 2.06 GB folder |
| **rbook** + html2text | – | – | – | 0.996 / 1.000 | – | 0/5 | 0 | 10/10 | – | 2 | 7 MiB | 4.4 MiB |

"Unreadable text" is the two scans plus the PDF without ToUnicode maps.
Sizes are unstripped binaries that already contain the app's libraries
(SQLite, Tokio); see Build cost for deltas. The lowest born-digital F1 is
the page with a table and footnotes, where footnote markers and the page
number count as extra words.

| Document | pdfium F1 / recall / order | pdf-extract | Docling |
| --- | --- | --- | --- |
| arXiv 2609.04872 | 0.91 / 0.94 / 0.73 | 0.94 / 0.96 / 0.71 | 0.96 / 0.95 / 0.63 |
| arXiv 2609.25046 | 0.93 / 0.95 / 0.80 | 0.97 / 0.97 / 0.85 | 0.97 / 0.97 / 0.66 |
| arXiv 2609.11975 | 0.98 / 0.98 / 0.99 | 0.84 / 0.77 / 0.98 | 0.99 / 1.00 / 0.98 |
| Federal Register 2026-22817 (three columns) | 0.93 / 0.99 / 0.99 | 0.93 / 0.99 / 0.99 | 0.93 / 0.98 / 0.97 |

Reading order against arXiv's HTML is a comparison between candidates,
not an absolute score: the HTML puts figures, tables and footnotes in
different places than the PDF does.

What stands out:

- **pdfium and Docling read the same text.** On the generated PDFs, where
  the truth is exact, all three PDF extractors score F1 0.99–1.00 and
  reading order 1.00, including two columns. On real papers the
  differences are small and go both ways. Docling's export leaves out
  running heads and page numbers, which raises its precision on some
  papers. It doesn't read papers in a better order than pdfium (0.87 vs
  0.90 against arXiv's HTML).
- **A PDF whose fonts lack ToUnicode maps fails silently.** pdfium and
  Docling return glyph ids as control characters, with no error.
  pdf-extract returns almost nothing. Only Atlas's own check above catches
  it, so Atlas needs that check whichever extractor it uses.
- **Ligatures.** pdf-extract leaves the ligature characters (ﬁ, ﬂ) in its
  output and splits the word around them (29 in this corpus). pdfium and
  Docling expand them.
- **pdf-extract missed a quarter of one paper** (recall 0.77 on
  arXiv 2609.11975).
- **Nothing crashed or hung**, on any malformed file, in any candidate.
  pdfium even recovered the full text of the PDF with the broken
  cross-reference table.
- **Docling reads EPUB too**, and finds every word (recall 1.00, order
  1.00). But it adds text that isn't in Gutenberg's plain edition (image
  captions and alternative text), so its F1 is 0.83 (0.44 on the
  illustrated Raven). It didn't notice the DRM file. It refused the
  64 MiB chapter with its own size limit, which rbook lacks.
- **EPUB is a solved problem with rbook.** F1 0.996 and chapter order
  1.000 against Gutenberg's own text, including Chinese and French. Two
  things are Atlas's job, not rbook's. DRM shows up only as
  `META-INF/encryption.xml`, which Atlas checks for. And a 64 MiB chapter
  made rbook use 336 MiB of memory, so Atlas must cap entry sizes before
  reading.

### On the CI runners

The scores are identical on macOS, Windows and Linux: the same text, to
the token, for every candidate and document. Nothing crashed or hung on
any OS. Throughput on the 3–4-core runners:

| Pages/s (44-page book) | CI macOS | CI Windows | CI Linux |
| --- | ---: | ---: | ---: |
| pdfium | 581 | 583 | 555 |
| pdf-extract | 551 | 543 | 481 |

### What Docling costs

Docling was frozen with PyInstaller (one folder, not one file: a single
file unpacks everything to a temporary folder on each start). It runs on
CPU, without OCR, with only the two models it loads (layout and
TableFormer).

| | M5 Pro |
| --- | ---: |
| Folder size | 2.06 GB, 8,127 files |
| Zipped | 982 MB |
| … of which models | 530 MB |
| PyInstaller build | 92 s (after a 1.1 GB `uv sync` and a 670 MB model download) |
| First launch after building | 22.6–24.9 s (two builds) |
| Later cold starts, models loaded | 3.0–3.3 s |
| Warm extraction | 0.16 s/page (44-page book), 0.46 s/page (27-page paper) |
| Peak memory | 3.8 GB |

On the CI runners:

| | CI macOS | CI Windows | CI Linux |
| --- | ---: | ---: | ---: |
| Folder size | 2.09 GB, 8,511 files | 1.46 GB, 9,507 files | 2.01 GB, 8,191 files |
| Zipped | 988 MB | 848 MB | 1,013 MB |
| PyInstaller build | 354 s | 221 s | 148 s |
| Cold starts, models loaded (5 runs) | 7.1–11.1 s | 8.1–8.2 s | 6.3–7.6 s |
| Warm extraction (44-page book) | 0.55 s/page | 1.1 s/page | 0.67 s/page |
| Peak memory | 3.0 GB | not measured | 3.1 GB |

Its text was identical to the M5 Pro's on all three OSes.

Freezing it took three fixes for things PyInstaller didn't collect on its
own: a data file of `latex2mathml`, transformers' lazily imported image
processors, and torchvision's native operators. Each of these broke the
build silently: it built, then failed at the first document. The frozen
program also re-launches itself for multiprocessing helpers, which needs
`multiprocessing.freeze_support()`. A Tauri sidecar must be a single
executable, so an 8,000-file folder would ship as resources instead, and
every file in it would need signing for notarized macOS builds.

## Vector engines

**Data.** 1,024-dimension unit vectors (the size of
`qwen3-embedding:0.6b`) drawn around random cluster centres, a rough
stand-in for text embeddings. Each vector belongs to one of 8 folders of
skewed sizes, so filters can be tested at about 40%, 5% and 0.5% of the
vectors. Exact top-10 neighbours for every query come from brute force.
Synthetic data like this is harder for graph indexes than real
embeddings; real embeddings are a Phase 2 measurement.

**Method.** Every engine runs the same scenario through one trait:

- build;
- close and reopen, timing the first answer;
- a sweep over its search setting, single-threaded, k = 10;
- filtered searches at the first setting that reaches recall 0.95;
- delete 1% and check none of them come back;
- compact.

A separate crash test kills an insert with SIGKILL after a few durable
steps and reopens the index. Engines are compared at matched recall,
never at their defaults.

### Results at 1,000,000 vectors on the M5 Pro

200 queries. Peak memory includes the harness's own copy of the data (3.8 GB), so an engine's own share is roughly the figure minus 3.8 GB: about 0.1 GB for sqlite-vec, 2.2 GB for usearch, 4–5 GB for LanceDB.

| Engine | Setting | Recall@10 | p95 | Filter 40% recall / p95 | Filter 5% | Filter 0.5% | Build | Disk ÷ raw | Open → first answer | Peak memory |
| --- | --- | ---: | ---: | --- | --- | --- | ---: | ---: | ---: | ---: |
| exact (harness) | brute force in Rust | 1.000 | 430 ms | 1.000 / 182 ms | 1.000 / 28 ms | 1.000 / 3.1 ms | – | 1.00 | 1.5 s | 12.2 GB |
| sqlite-vec | f32, published crate | 1.000 | 997 ms | 1.000 / 403 ms | 1.000 / 50 ms | 1.000 / 5.0 ms | 178 s | 1.01 | 0.97 s | 3.8 GB |
| sqlite-vec | f32, L2, NEON | 1.000 | 517 ms | 1.000 / 203 ms | 1.000 / 26 ms | 1.000 / 2.4 ms | 180 s | 1.01 | 0.50 s | 3.8 GB |
| sqlite-vec | int8 (scaled), L2, NEON | 0.954 | 157 ms | 0.958 / 66 ms | 0.972 / 7.9 ms | 0.979 / 0.8 ms | 30 s | 0.26 | 0.16 s | 3.8 GB |
| sqlite-vec | bit, rescore 400 | 0.833 | 244 ms | 0.999 / 103 ms | 1.000 / 14 ms | 0.898 / 2.4 ms | 15 s | 1.16 | 0.24 s | 3.8 GB |
| **usearch** | HNSW f16, ef 128 | 0.963 | **0.84 ms** | 0.984 / 1.0 ms | 0.988 / 6.8 ms | 0.988 / 54 ms | 47 s (18 threads) | 0.54 | 0.50 s | 6.0 GB |
| usearch | HNSW i8 (unscaled), ef 512 | 0.672 | 0.83 ms | 0.716 / 1.3 ms | 0.836 / 12 ms | 0.887 / 100 ms | 20 s | 0.29 | 0.30 s | 5.1 GB |
| LanceDB | IVF-PQ, nprobes 100, refine 100 | 1.000 | 24.5 ms | 1.000 / 31 ms | 1.000 / 24 ms | 0.970 / 20 ms | 12 s | 1.02 | 48 ms | 7.7 GB |
| LanceDB | IVF-HNSW-SQ, ef 320 | 0.953 | 0.86 ms | 0.971 / 9.1 ms | 0.970 / 17 ms | 0.982 / 7.5 ms | 70 s | 1.30 | 0.21 s | 8.9 GB |

Recall–latency sweeps (recall@10 / p95):

- **usearch f16, ef 16 → 512:** 0.556 / 0.26 ms · 0.700 / 0.39 · 0.873 / 0.59 · 0.963 / 0.84 · 0.981 / 0.98 · 0.992 / 1.27 ms.
- **LanceDB IVF-PQ, (nprobes, refine) from (5, 2) to (100, 100):** 0.100 / 2.1 ms · 0.192 / 3.3 · 0.312 / 5.7 · 0.488 / 6.4 · 0.488 / 11.6 · 0.817 / 13.7 · 1.000 / 24.5 ms.
- **LanceDB IVF-HNSW-SQ, ef 10 → 320:** 0.598 / 0.58 ms · 0.785 / 0.60 · 0.894 / 0.68 · 0.943 / 0.71 · 0.948 / 0.76 · 0.953 / 0.86 ms.
- **sqlite-vec bit, rescoring 10 → 400 candidates:** 0.098 / 22 ms · 0.159 / 26 · 0.298 / 43 · 0.431 / 71 · 0.618 / 127 · 0.833 / 244 ms.

**On the CI runners at 100,000 vectors** (200 queries), usearch f16
reaches recall 0.958–0.965 at ef 64 in 0.31–0.49 ms on all three OSes.
LanceDB IVF-PQ needs 9–23 ms. sqlite-vec int8 brute force takes 26 ms with
NEON (CI macOS) but 88–110 ms on x86. On Windows, MSVC builds sqlite-vec
without its AVX code, so the "SIMD" build barely helps there.

**At 100,000 vectors on the M5 Pro** (1,000 queries) the picture is the
same at a tenth of the scale:

- usearch f16 reaches 0.991 at ef 128 in 0.35 ms;
- sqlite-vec int8 with NEON takes 16 ms;
- LanceDB uses about 3 GB of memory;
- the crash test results above come from this size.

What stands out:

- **Only the graph and partition indexes meet the latency target at a
  million vectors.** usearch reaches recall 0.96 in 0.84 ms. LanceDB's
  IVF-PQ needs `nprobes` 100 and a 100× refine step to reach 1.00 (24.5 ms):
  its default product quantization loses too much on 1,024 dimensions.
- **sqlite-vec is brute force**, so whole-library search is too slow. A
  query reads every vector: 0.52 s in f32 even with SIMD, 0.16 s in int8.
  Folder-scoped search is fast thanks to the partition key (25 ms at 5%).
- **The published sqlite-vec crate turns off SIMD.** Its build script
  compiles none of the NEON/AVX code, and cosine distance has no SIMD path
  at all. Built with NEON and using L2 (equivalent on unit vectors), queries
  are 1.9× faster (int8: 4.3×). With MSVC on Windows the AVX code doesn't
  compile, so it stays scalar.
- **The sqlite-vec 0.1.10 alpha didn't work for us.**
  - The published crate doesn't build: it leaves out `sqlite-vec-diskann.c`
    and the other included files. We built it from the tag's sources.
  - Its DiskANN index returned recall 0.002–0.015 at every search-list
    size, built 100k vectors in 104 s at 3.6× the raw size, and took 41 s
    to delete 1%.
  - It may be a mistake on our side; the alpha's own tests ask only for 3
    of the top 5. Either way it's an alpha and can't be adopted.
- **int8 needs a scale.** Both sqlite-vec's `vec_quantize_int8(v, 'unit')`
  and usearch's i8 assume components near ±1, but those of a 1,024-d unit
  vector are about ±0.03, which leaves a handful of levels. Scaled by the
  data's largest component, sqlite-vec's int8 reaches recall 0.95; usearch's
  i8 isn't adjustable and stays at 0.67.
- **Durability differs.** SQLite and LanceDB always came back consistent
  after `kill -9`. usearch's `save` rewrites the whole file in place. On the
  M5 Pro the kill landed during a save and left an index that no longer
  loads ("Failed to read vectors"). On the three CI runners it landed
  between saves, and the index was fine. That is detectable and the index
  can be rebuilt, but an app must save to a temporary file and rename it.
- **Deletes.** All four honour deletes immediately. usearch leaves
  tombstones that only a rebuild reclaims, and it rewrites its file to make
  them durable (1.1 s at 1M). LanceDB writes a deletion file in
  milliseconds.

## Build cost

Each binary was built in its own fresh target directory, after `cargo
fetch`, in the release profile. Deltas are against a baseline that links
what the app already links (bundled SQLite, Tokio, serde_json,
unicode-normalization), so they are what adopting the candidate would add.

| Candidate | Clean build (M5 Pro) | Δ build | Stripped binary | Δ stripped | Shipped beside it |
| --- | ---: | ---: | ---: | ---: | --- |
| baseline (extraction) | 13 s | – | 2.4 MiB | – | |
| pdfium | 14 s | +1 s | 5.0 MiB | +2.6 MiB | PDFium library: 7.0 MiB (macOS), 7.2 MiB (Windows), 7.7 MiB (Linux) |
| pdf-extract | 14 s | +1 s | 3.7 MiB | +1.3 MiB | |
| rbook + html2text | 15 s | +2 s | 4.0 MiB | +1.6 MiB | |
| Docling | – | PyInstaller 92 s | – | – | 2.06 GB folder, 982 MB zipped |
| baseline (vectors) | 12 s | – | 2.4 MiB | – | |
| sqlite-vec | 12 s | +0 s | 2.6 MiB | +0.2 MiB | |
| usearch | 24 s | +12 s | 4.1 MiB | +1.8 MiB | |
| LanceDB | 278 s | +266 s | 195.0 MiB | +192.6 MiB | needs `protoc` to build |

On the CI runners (3–4 cores):

| Δ over baseline | CI macOS | CI Windows | CI Linux |
| --- | ---: | ---: | ---: |
| pdfium | −2 s, +2.6 MiB | +24 s, +1.2 MiB | +9 s, +3.0 MiB |
| rbook + html2text | +7 s, +1.6 MiB | +28 s, +1.9 MiB | +11 s, +1.9 MiB |
| sqlite-vec | +5 s, +0.2 MiB | +1 s, +0.2 MiB | +1 s, +0.2 MiB |
| usearch | +22 s, +1.7 MiB | +107 s, +2.6 MiB | +103 s, +4.0 MiB |
| **LanceDB** | **+32 min, +193 MiB** | **+45 min, +254 MiB** | **+25 min, +223 MiB** |

Small negative deltas are noise. Windows binaries carry no symbols to
strip (they go to a separate `.pdb`), so their stripped and unstripped
sizes are equal.

## Licenses

All candidates are permissive. The details are in
[spike-dependency-licenses.md](spike-dependency-licenses.md):

- **PDFium** is BSD-3-Clause and ships third-party code (FreeType,
  libjpeg-turbo, OpenJPEG, Little CMS, zlib and others) whose notices must
  ship with it.
- **Docling's** code is MIT. Its layout model is Apache-2.0, and TableFormer
  is CDLA-Permissive-2.0. Its Python environment includes three MPL-2.0
  packages (`certifi`, `tqdm`) and PyInstaller, which is GPL with an
  exception that lets frozen programs be distributed under any license.
- **LanceDB** brings in `option-ext` (MPL-2.0).
- **usearch and sqlite-vec** are Apache-2.0 and MIT OR Apache-2.0.

MuPDF and PyMuPDF (AGPL) and the `epub` crate (GPL-3.0) were excluded
before measuring.

## Not measured

- **Real embeddings** (no model was downloaded) and recall on real text;
  Phase 2 measures both.
- **OCR quality.** Scanned pages are only detected here; the reference text
  for the synthetic scan and the 1894 Alice scan is in the corpus for
  later.
- **Windows on ARM, Linux on ARM, and Intel Macs.**
- **Docling with OCR or a GPU.**
- **Statically linked PDFium** (no prebuilt static libraries exist; it
  would mean building PDFium).
