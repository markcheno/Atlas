# Slice 4.2 spike: the OCR engine

These are measurements, not guarantees (phase4-spec §10.1). They inform
[ADR 0020](adr/0020-ocr-engine.md), whose candidates, measures and rule
were committed (`004894d`) before anything was measured. The harness is
in [`spikes/ocr-bench/`](../spikes/README.md) and is not part of the app.

**Status: measured in part, then deferred.** On 2026-11-13 the product
owner deferred OCR to a later version: their collection has few scans
(phase4-spec revision notes). The spike was closed with what had been
measured, and ADR 0020 stays Proposed with its criteria intact.

**What was measured:** Tesseract's accuracy and speed over the corpus on
the M5 Pro; every engine's clean build on three OSes; every variant inside
Atlas's real installers on three OSes, with its check from the installed
place. **What wasn't:** PP-OCR's and Vision's accuracy over the corpus (the
M5 Pro's run was stopped to free the owner's Mac for indexing, and the CI
corpus build stopped at a pin, see below), and the owner's own scans.

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

### Accuracy and speed: Tesseract on the M5 Pro (2026-11-13)

All 50 pages, one process each, one thread. WER and CER are the mean of
the pages' rates; by category, WER. Seconds a page are after the first
page; peak memory is the process's over the whole pass.

| Variant | DPI | WER | CER | Book | Two-column | Degraded | Poor layer | Reading order | s/page (median, p90) | Peak memory | Failed |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| tesseract-fast | 300 | 2.9% | 2.2% | 3.6% | 1.9% | 3.8% | 1.9% | 0.997 | 0.43, 2.14 s | 556 MiB | 0 |
| tesseract-fast | 200 | 2.6% | 1.8% | 3.5% | 1.8% | 2.9% | 1.4% | 0.997 | 0.32, 2.07 s | 344 MiB | 0 |
| tesseract-best | 300 | 2.7% | 2.0% | 2.9% | 1.9% | 3.7% | 1.8% | 0.996 | 1.01, 6.33 s | 502 MiB | 0 |
| tesseract-best | 200 | 2.5% | 1.7% | 3.1% | 1.8% | 2.9% | 1.2% | 0.997 | 0.96, 5.66 s | 359 MiB | 0 |

- **The poor-layer pages' own OCR text** scores 97.1% WER and 82.9% CER
  against the reference, mostly lines out of order: Tesseract reads the
  same pages at under 2%.
- **`best` against `fast`** at 300 DPI: 0.28 points lower WER (95%
  interval 0.06 to 0.49), for 2.4 times the time a page and 10 MiB more
  in each installer. At 200 DPI the difference is inside the interval.
- **200 DPI did as well as 300** on this corpus, and faster: the scans are
  200 to 587 ppi, and Tesseract estimates the text size itself.
- **The p90 is the two-column encyclopaedia pages**, 9,000 to 11,000
  characters each.

### Build cost (CI, Spikes run [43363368422](https://github.com/markcheno/Atlas/actions/runs/43363368422))

A clean release build of each engine binary in a fresh target folder,
after the sources are fetched; against the `baseline` binary, which renders
pages and links what the app links.

| Engine | macOS arm64 | Windows x64 | Linux x64 |
| --- | --- | --- | --- |
| Tesseract (built from source): build time, binary | +87 s, +2.5 MiB | +117 s, +3.8 MiB | +88 s, +4.9 MiB |
| PP-OCR (oar-ocr, ONNX Runtime loaded at run time): build time, binary | +175 s, +1.2 MiB | +226 s, +2.0 MiB | +110 s, +1.9 MiB |
| Vision: build time, binary | none (−19 s, +0.1 MiB) | – | – |

Every engine built on all three OSes with only downloaded tools (cmake
from `spike-tools`).

### Installers (CI, Spikes run [43363376670](https://github.com/markcheno/Atlas/actions/runs/43363376670))

Each installer carrying the variant's engine as a stand-in, with its model
files and, for PP-OCR, ONNX Runtime; less the same installer carrying the
baseline stand-in. Every variant's check passed from every installed
layout, three times each: the signed `.app`, a per-user Windows install,
the extracted `.deb` and the AppImage.

| Variant | macOS DMG | Windows NSIS | Linux `.deb` | Linux AppImage |
| --- | --- | --- | --- | --- |
| tesseract-fast | +3.1 MiB | +2.9 MiB | +4.3 MiB | +4.1 MiB |
| tesseract-best | +13.4 MiB | +13.1 MiB | +14.6 MiB | +14.4 MiB |
| ppocr-v5-mobile | +22.1 MiB | +15.6 MiB | +20.8 MiB | +19.7 MiB |
| ppocr-v6-tiny | +16.2 MiB | +9.8 MiB | +15.0 MiB | +13.9 MiB |
| ppocr-v6-small | +35.6 MiB | +28.9 MiB | +34.5 MiB | +33.2 MiB |
| vision-accurate-raw | +0.0 MiB | – | – | – |

Every PP-OCR variant is within the rule's 40 MiB; v6 small comes closest
(35.6 MiB on macOS). The baselines are 16.6 MiB (DMG), 11.8 MiB (NSIS),
20.6 MiB (`.deb`) and 93.5 MiB (AppImage).

### What the rule could and couldn't say

The rule needs PP-OCR's WER on every OS beside Tesseract's, which wasn't
measured. But the Tesseract candidate (the better of its two variants) has
a corpus WER of 2.7% at 300 DPI, and PP-OCR is chosen only if its WER is
at least 3 points lower: below zero, which no engine can reach. **On this
corpus the rule can only choose Tesseract**, whatever PP-OCR measures;
Vision is in the same position on macOS. A 3-point margin over an engine
already under 3% means the rule, as written, will keep Tesseract on any
corpus this clean. This is recorded as what the measurements
imply, not as a decision; ADR 0020's decision waits for OCR's return.

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

Not run: OCR was deferred first. `just spike-ocr-mine <folder>` is ready
for when it returns.

## When OCR returns

1. **The corpus's scans must come from the originals.** CI's corpus build
   stopped at its first page: Commons served a re-rendered JPEG for a scan
   the manifest pinned (`make: … the scan changed`), as it may for any of
   its thumbnails. Fetch the DjVu or the Internet Archive's JPEG 2000
   originals (Pillow reads JPEG 2000), or commit the public-domain scans.
2. **Measure PP-OCR and Vision over the corpus** on each OS, then apply
   ADR 0020's rule.
3. **The worker's render** must use BGRA with PDFium's greyscale flag
   (PDFium's grey format leaves scans undrawn).
4. **A memory cap** must allow for PP-OCR's 1 to 4 GiB a page, if it is
   chosen.

## Licences

[docs/spike-dependency-licenses.md](spike-dependency-licenses.md), by
`just spike-licenses`.
