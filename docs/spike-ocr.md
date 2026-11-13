# Slice 4.2 spike: the OCR engine

These are measurements, not guarantees (phase4-spec §10.1). They inform
[ADR 0020](adr/0020-ocr-engine.md), whose candidates, measures and rule
were committed (`004894d`) before anything was measured. The harness is
in [`spikes/ocr-bench/`](../spikes/README.md) and is not part of the app.

**Status:** harness, corpus and engines built; the measurements are to be
recorded here, then the rule applied in ADR 0020.

## The question

Atlas reads scanned PDFs from Slice 4.3. Which engine, read inside the
extraction worker one page at a time at 300 DPI, gives the best text for
English scans, within what each installer can carry, on all three OSes?

- **Tesseract 5.5** with Leptonica, built from source and linked into the
  engine, with Tesseract's own English models (`fast` and `best`).
- **PP-OCR** (PaddleOCR's detection and recognition models) through ONNX
  Runtime: PP-OCRv5 mobile with its English recogniser, and PP-OCRv6 tiny
  and small (the product owner added v6 on 2026-11-12).
- **Apple's Vision framework**, on macOS only: nothing to ship.

## Machines

| Label | Machine |
| --- | --- |
| **M5 Pro** | MacBook Pro (Mac17,9), Apple M5 Pro, 18 cores, 64 GB, macOS 27 |
| **CI** | GitHub-hosted runners: macOS arm64, Windows Server 2025 x64, Ubuntu 24.04 x64 |

## The corpus

About 50 English pages, each an image-only PDF page that the engines
render with PDFium as the worker will. Pages are chosen by
`generator/corpus.py select` and pinned in `corpus/manifest.json`; `make`
fetches exactly what is pinned and fails on any change.

| Category | Pages | Source | Reference text |
| --- | --- | --- | --- |
| Book scans | 20 | 4 pages from each of 5 books (1883–1913): *Alice's Adventures in Wonderland* (Cassell, 1907), *The Time Machine* (Heinemann, 1895), *Treasure Island*, *The Jungle Book*, *The Wind in the Willows* (1913). Commons' render of the scan at its full width and resolution | Wikisource's validated transcription of the page (pinned by revision) |
| Two-column | 15 | 10 pages of the 1911 *Encyclopaedia Britannica* (text articles only), and 5 two-column "papers" of our own, typeset from Project Gutenberg text with a light print-and-scan look | Wikisource; for our own, the exact text |
| Degraded | 10 | 2 further pages of each book, each through one seeded degradation: 150 and 100 ppi, a 2–3° skew, speckle, JPEG at quality 20–30, blur, binarised, and all at once | Wikisource |
| Poor text layer | 5 | Pages whose scan file carries its own hidden OCR text with a word error rate of at least 10% against the reference | Wikisource; the file's own layer is scored as the `text-layer` row |

- **Wikisource's text is CC BY-SA 4.0**, so it is fetched at build time
  and never committed. The text is the page as Wikisource renders it
  (templates expanded by Wikisource itself), including the running head
  and the page number, which every engine also sees.
- **CI builds the corpus once, on Linux**, and every OS's job reads that
  build, so the pages are byte-identical everywhere.
- **Deviations from the plan, decided while building it:**
  - Slice 0.9's 1894 *Alice* scan isn't used: it has no reference text,
    and it was stored at 130 ppi. Every book page has Wikisource's
    validated, page-aligned text instead.
  - Slice 0.9's synthetic scan isn't used: its reference covers its three
    pages together, and here every page is scored on its own. The ten
    degraded pages all come from validated pages.
  - The "poor text layer" pages are scans whose DjVu file carries its own
    OCR text (the Internet Archive's, in these books); that text is the
    reference row the engines are compared with. Only *The Time Machine*'s
    layer was poor enough (word error rate at least 10%): its five pages
    score about 0.9 to 1.0, mostly from lines out of order. The other
    books' layers were better than the threshold.
  - **Scans aren't all 300 ppi or more**, as the plan wanted. Commons
    serves a page at a standard width no larger than the scan's (1920 px
    for most of these books, 3840 for *Treasure Island*), so the images
    are 200 to 587 ppi. Each PDF page is sized by its image's real
    resolution (`ppi` in the manifest, beside the scan's own), so pages
    keep their printed size and the 300 DPI render scales them as the
    worker would scale a scan of that resolution.

## How it was measured

[ADR 0020](adr/0020-ocr-engine.md) holds the definitions: the
normalisation, WER and CER by Levenshtein, the mean of the pages' rates
as the corpus figure, 300 DPI for the rule, and the bootstrap interval
reported beside each difference.

- **Rendering:** PDFium at the DPI asked for, annotations off, greyscale.
  PDFium's own grey bitmap format leaves image objects undrawn (which is
  all a scan is), so pages are rendered BGRA with PDFium's greyscale flag
  and reduced to one byte a pixel (BT.601 weights). Slice 4.3's worker
  must do the same.
- **One thread:** ONNX Runtime's pools at 1 with spinning off, and rayon at
  1 (PP-OCR); Tesseract is built without OpenMP. CPU time is reported
  beside wall time. Vision chooses its own threads.
- **PP-OCR's detection settings** are each model's own defaults, from its
  `inference.yml` (v5: 0.3, 0.6, unclip 1.5; v6: 0.2, 0.45 or 0.4 for
  tiny, unclip 1.4). Lines from PP-OCR and Vision are put in reading order
  by one column-ordering function (an XY cut), shared by both.
- **Speed:** each variant reads the whole corpus in one process; the first
  page is reported apart. The time from a new process to its first page
  is the median of five runs.
- **Installers:** each variant's engine binary is bundled into Atlas's
  real installers as a stand-in, with its files, by `pnpm tauri build
  --config`; its size is compared with the same installers carrying the
  baseline stand-in. From each installed layout, `ocr-standin
  --self-check` finds PDFium, ONNX Runtime and the models by the worker's
  rules and reads one scanned page (CER at most 5%), three times.
- **Build cost:** a clean release build of each engine binary in a fresh
  target folder, after the sources are fetched.

## Results

To be recorded from the M5 Pro's run and the CI runs.

### Accuracy (300 DPI)

### Speed and memory

### Installers

First measured locally (M5 Pro, 2026-11-12): the macOS DMG with the
`tesseract-fast` stand-in and its model is 3.1 MiB larger than with the
baseline stand-in, and the stand-in passed its check twice from the
installed `.app`. The full table comes from the `ocr-installers` suite.

### Build cost

### Seen while building the harness

Single pages, not measurements under the rule; recorded so they aren't
lost:

- **PP-OCR's memory:** 0.8 to 1.2 GiB at most for one letter-size page at
  300 DPI, and up to 3.9 GiB on a larger scanned page, against 0.1 to
  0.25 GiB for Tesseract and Vision. The detector works on the whole page
  at once. A memory cap for the worker (§7.2) would have to allow for it.
- **Vision's first use on a machine** took 26 s (its models loading); the
  next process took 0.2 s.
- **Tesseract reports 5.5.3**, although the crate's notes say 5.5.4.

## The owner's own scans

`just spike-ocr-mine <folder>`: to be recorded when the owner runs it.

## Licences

[docs/spike-dependency-licenses.md](spike-dependency-licenses.md), by
`just spike-licenses`.
