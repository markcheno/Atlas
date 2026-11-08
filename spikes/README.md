# Spikes

Experiments that inform decisions but are **not part of the app**. Nothing
here is built by `just ci`, `pnpm tauri build` or `ci.yml`; the app's
`src-tauri` crate doesn't depend on anything in this folder.

## Slice 0.9: extraction and vector engines

Results: [docs/spike-0.9.md](../docs/spike-0.9.md). Decision:
[ADR 0008](../docs/adr/0008-extraction-and-vector-engines.md). Licenses:
[docs/spike-dependency-licenses.md](../docs/spike-dependency-licenses.md).

| Folder | What |
| --- | --- |
| `extraction-bench/` | Cargo workspace: one binary per extraction candidate (`pdf-pdfium`, `pdf-extract`, `epub-rbook`), a `baseline` for size and build-time deltas, and `score` |
| `extraction-bench/corpus/` | The committed spike corpus (≈9 MiB): public-domain and CC BY 4.0 documents, generated PDFs with exactly known text, scans, malformed files. `manifest.json` records each file's source, license and SHA-256 |
| `extraction-bench/generator/` | `fetch.py` (downloads the redistributable sources) and `make.py` (typesets and generates the rest); a uv project |
| `extraction-bench/sidecar/docling/` | Docling frozen into a standalone folder with PyInstaller (`build.py`), speaking the same JSON contract as the Rust candidates |
| `vector-bench/` | Cargo workspace: `datagen` (synthetic 1024-d vectors with exact ground truth), one binary per engine (`vec-sqlite`, `vec-sqlite-simd`, `vec-sqlite-alpha`, `vec-usearch`, `vec-lancedb`) and a brute-force `baseline` |
| `scripts/` | Drivers: `fetch-tools`, `check-corpus`, `run-extraction`, `run-vectors`, `measure-builds` |
| `tools.lock.json` | Pinned downloads (PDFium, protoc, typst) with SHA-256 per OS |
| `.tools/` | Where `fetch-tools` unpacks them (gitignored) |

## Running

```sh
just spike-tools                 # PDFium, protoc, typst into spikes/.tools (checksummed)
just spike-corpus                # verify the corpus
just spike-extract               # pdfium, pdf-extract, rbook on the corpus
just spike-docling               # freeze Docling (downloads ~2 GB: PyTorch, models)
just spike-extract "" spikes/extraction-bench/sidecar/docling/build/dist/atlas-docling/atlas-docling
just spike-vectors 100000        # every engine at 100k vectors (1000000 needs ~20 GB free)
just spike-builds                # clean build time and binary size per candidate
just spike-licenses              # docs/spike-dependency-licenses.md
```

Results go to the system temp folder (`atlas-spikes/`) unless `out` is
given. CI runs the same scripts on macOS, Windows and Linux when the
**Spikes** workflow is dispatched by hand (`.github/workflows/spikes.yml`).

Requirements beyond the app's: [uv](https://docs.astral.sh/uv/) for the
generator and the Docling sidecar, and a C++17 compiler (usearch).
Everything else is downloaded into `spikes/.tools` by `just spike-tools`.

## The extractor contract

Every extraction candidate takes file paths as arguments and writes one
JSON line per file to `$XBENCH_OUT` (else stdout), then a `done` line:

```json
{"file": "…", "outcome": "ok", "pages": ["text of page 1", "…"], "ms": 3.1}
{"done": true, "extractor": "pdfium", "version": "…", "peakRssBytes": 28311552}
```

`outcome` is `ok`, `encrypted`, `malformed`, `unsupported` or `panic`; the
driver adds `crash` and `timeout`. EPUB "pages" are spine documents.
