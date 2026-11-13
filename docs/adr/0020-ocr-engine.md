# ADR 0020: The OCR engine

- **Status:** Proposed. These criteria are committed before any candidate
  is measured (Slice 4.2); the decision will follow from the measurements
  and be recorded here, with the commit that holds these criteria.
- **Date:** 2026-11-12
- **Context documents:** [Phase 4 spec](../phase4-spec.md) §3, §7.2, §10.1
  and its revision notes for Slice 4.2; [ADR 0008](0008-extraction-and-vector-engines.md)
  (the extraction spike this follows); results will be in
  [the OCR spike](../spike-ocr.md).

## Context

Phase 4 reads scanned PDFs: documents the Phase 1 reader marked Needs OCR
(no text layer, or text that is garbled). The engine runs inside the
extraction worker, under a supervisor of its own, one page per request,
on page images PDFium renders at 300 DPI in greyscale (§3, §7.2). It ships
inside every installer, reads English only, and runs in the background
under the resource policy.

The engine is hard to change later. It decides what each installer
carries, how the build is set up on three OSes, how fast scans are read
and how well they can be searched. So it is chosen by a spike, in the
manner of Slice 0.9, by a rule written here before anything is measured.
The harness is in [`spikes/ocr-bench/`](../../spikes/README.md) and is not
part of the app.

## Candidates

| Engine | How it would ship | Licence |
| --- | --- | --- |
| Tesseract 5.5 with Leptonica, built from source (`xberg-tesseract`) | Linked into the worker; an English model file | Apache-2.0; Leptonica BSD-2; the crate MIT; models Apache-2.0 |
| PP-OCR (PaddleOCR's detection and recognition models) through ONNX Runtime (`ort`, `oar-ocr`) | ONNX Runtime as a shared library the worker loads, as it loads PDFium; model files | Models Apache-2.0; ONNX Runtime MIT; the crates MIT/Apache-2.0 and Apache-2.0 |
| Apple's Vision framework (`VNRecognizeTextRequest`, through `objc2-vision`) | Part of macOS: nothing to ship. macOS only | System framework; the crate Zlib/Apache-2.0/MIT |

**Variants measured.** An engine's variants share its code and differ in
their model files:

| Variant | Model files |
| --- | --- |
| `tesseract-fast` | `eng.traineddata` from `tessdata_fast` |
| `tesseract-best` | `eng.traineddata` from `tessdata_best` |
| `ppocr-v5-mobile` | PP-OCRv5 mobile detection; the English-only PP-OCRv5 mobile recogniser |
| `ppocr-v6-tiny` | PP-OCRv6 tiny detection and recognition (multilingual: v6 has no English-only recogniser) |
| `ppocr-v6-small` | PP-OCRv6 small detection and recognition |
| `vision-accurate-lc` | Accurate level, `en-US`, language correction on |
| `vision-accurate-raw` | The same, language correction off |

PP-OCRv6 (June 2026) is measured beside v5 by the product owner's decision
of 2026-11-12. `text-layer`, the PDF's own text where it has one, is a
reference row, not a candidate. `pure-onnx-ocr` (the PP-OCR models on the
pure-Rust `tract` runtime) is measured for information only: it isn't
ONNX Runtime.

**Excluded before measuring:**

- `ocrs`: its models are CC BY-SA 4.0.
- PP-OCRv5 server models (about 172 MB) and PP-OCRv6 medium (the
  recogniser alone is 76.6 MB): over the size limit by their models alone.
- Engines that need Python at run time: no installer carries Python.
- Vision-language models: deferred by §2.

## Decision criteria

Fixed before measuring.

### Hard gates

A variant that fails one can't be chosen, whatever its accuracy.

- **Licences:** permissive for everything that would ship, including
  transitive dependencies and models.
- **Builds** on macOS arm64, Windows x64 and Linux x64 with only
  downloadable tools: no system-wide installs, no system libraries beyond
  what Atlas already needs.
- **Passes the worker check in each installed layout** (the macOS app,
  the Windows installer, the Linux `.deb` and AppImage). In this slice the
  check is a stand-in: a small program built with the engine, bundled into
  a real Atlas installer beside the app, which finds its library and model
  files from its installed place, by the worker's own rules, and reads one
  scanned page with a character error rate of at most 5%, three times.
  The real worker with the engine inside is Slice 4.3's check (§14).
- **No crash or hang**, and no page over 60 s (§7.2's limit), on the
  corpus.
- **For PP-OCR:** each installer grows by at most 40 MiB, measured on the
  real installers (DMG, NSIS, `.deb`, AppImage), engine code, ONNX Runtime
  and models together.

### How accuracy is measured

- **The corpus:** about 50 English pages with reference text, in four
  categories: 20 book scans, 15 two-column pages, 10 degraded pages and 5
  scans that already carry a poor OCR text layer. Every page is an
  image-only PDF page rendered by PDFium, as the worker renders it. The
  corpus is frozen (its manifest committed) before any engine runs.
- **Normalisation**, of both the engine's text and the reference:
  - NFKC;
  - curly quotes and primes to ASCII; en and em dashes and the minus sign
    to `-`;
  - soft hyphens and zero-width characters removed;
  - words broken across lines with a hyphen joined;
  - lower case;
  - runs of whitespace to one space.
- **Word error rate (WER):** the Levenshtein distance (substitutions,
  insertions and deletions) between the engine's words and the
  reference's, divided by the number of reference words. Words are the
  alphanumeric runs Slice 0.9's scorer uses.
- **Character error rate (CER):** the same over characters, punctuation
  kept.
- **Both depend on order**, deliberately: the order of the text is what
  Atlas stores and what passages are made of. An order-free word error
  (1 − word F1) and Slice 0.9's reading-order measure are reported beside
  them, to tell a layout error from a misread.
- **The corpus figure** is the mean of the pages' WERs, so each category
  keeps the weight the corpus gives it. The WER over all words pooled is
  reported too.
- **At 300 DPI** (§3). 200 DPI is measured and reported, and the decision
  says whether it would have come out differently.
- **Box engines** (PP-OCR, Vision) return lines with positions; their
  lines are put in reading order by one column-ordering function, the
  same for both, which is part of the engine as Slice 4.3 would ship it.
  Tesseract orders its own output.

### The rule

1. **The Tesseract candidate** is the better (lower corpus WER) of
   `tesseract-fast` and `tesseract-best`.
2. **The PP-OCR candidate** is the best of `ppocr-v5-mobile`,
   `ppocr-v6-tiny` and `ppocr-v6-small` among those that pass every hard
   gate, the 40 MiB limit included.
3. **Tesseract is chosen** unless the PP-OCR candidate's corpus WER is at
   least 3 points lower than the Tesseract candidate's on every OS where
   both ran. Then PP-OCR is chosen.
4. **Apple's Vision framework is used on macOS** only if the better of its
   two variants has a corpus WER at least 3 points lower than the chosen
   engine's on macOS. If it is used, text read on macOS will differ from
   text read elsewhere, and Vision changes with macOS updates where
   Atlas's OCR version can't see it; the decision records both.
5. **If no candidate passes the hard gates on all three OSes,** the slice
   stops and reports. OCR does not ship on some OSes only without the
   product owner's decision.

**Recorded, not deciding** (the product owner's decisions of 2026-11-12):

- **Uncertainty:** a 95% confidence interval for each WER difference, by a
  paired bootstrap over pages stratified by category (10,000 resamples, a
  fixed seed). With about 50 pages a 3-point difference may sit inside it;
  the interval is shown so a borderline result is visible, and the rule
  above still decides.
- **Speed:** seconds per page on one thread at 200 and 300 DPI (render and
  recognition apart, median and 90th percentile), the time to the first
  page in a new worker, and peak memory. Only a page over 60 s counts
  against a variant.
- **Build cost:** the clean build's time on each OS.
- **Agreement across OSes:** the CER between one variant's text on two
  OSes, which informs the floor §10.2 sets for Slice 4.3.
- **The owner's own scans** (`just spike-ocr-mine`): each engine's speed
  and text on 5 to 10 of the owner's scanned PDFs, read side by side. It
  has no reference text and is a check on how the engines behave, not part
  of the arithmetic.

## Decision

To be recorded from the measurements.

## Consequences

To be recorded with the decision.
