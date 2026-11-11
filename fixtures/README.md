# Test fixtures

Atlas never ships or commits real books or papers. Test libraries are
generated on demand (`scripts/generate-fixtures.mjs`, Node built-ins only),
either from tiny identical stubs or with synthetic body text:

```sh
just fixtures ~/atlas-fixtures/small                 # 1,000 PDF/EPUB stubs (1,006 catalogable)
just fixtures ~/atlas-fixtures/large 10000 20000     # the 30k benchmark set: 10,000 nested files
                                                     # plus one flat folder of 20,000 (30,006 catalogable)
just fixtures ~/atlas-fixtures/text 1000 0 20        # the same small layout with about 20 KiB of text per file
```

## Body text (`--text-kb N`)

Without `--text-kb` (or with 0) the output is the Phase 0 output, byte for
byte: every PDF is the same one-page stub and every "EPUB" is a placeholder,
so a whole fixture is one document by content hash. A test
(`scripts/generate-fixtures.test.mjs`) checks this against a hash recorded
before Phase 1 changed the generator.

With `--text-kb N`, every PDF and EPUB gets about N KiB of its own text
(phase1-spec §11):

- **Text** is drawn from a fixed word list by a seeded generator, so the
  same arguments give the same bytes on every machine. Each file's first
  sentence carries a unique **marker phrase**, `<subject> marker <i>` (for
  example `Astronomy marker 42`, or `Flat marker 7` in the flat folder),
  and topic phrases recur per subject (`orbital resonance`, `lock free
  queue`, …) along with a few shared ones (`quiet harbour lights`,
  `café crème`), so search questions over a fixture are derivable.
- **PDFs** have about 3,000 characters a page (Helvetica, WinAnsi), an Info
  dictionary with Title and Author, and a correct cross-reference table.
- **EPUBs** are valid EPUB 3 zips: `mimetype` first and stored, a container,
  an OPF with `dc:identifier`, `dc:title`, `dc:creator` and `dc:language`,
  a navigation document and four XHTML chapters. Entries are stored rather
  than deflated, because Node's zlib output differs between CPUs and
  versions and the fixtures must be reproducible.
- The six `duplicates/Introduction.pdf` files share one text, so they are
  duplicates by content as well as by name. `misc/`, the outside folder and
  the locked folder keep their stubs.
- The summary adds `textKb`, `pdfPages` and `epubChapters` (pages and
  chapters of distinct documents).

Measured on an M5 Pro (2026-11-08):

| Fixture | Arguments | Time | Size | PDF pages |
| --- | --- | ---: | ---: | ---: |
| 1k, 20 KiB | `1000 0 20` | 0.4 s | 26 MiB | 6,668 |
| 1k, 100 KiB | `1000 0 100` | 1.7 s | 119 MiB | 31,343 |
| 30k, 20 KiB (CI) | `10000 20000 20` | 12 s | 808 MiB | 266,598 |
| 30k, 100 KiB | `10000 20000 100` | 49 s | 3.6 GiB | 1,253,610 |

### Titles and copies (Slice 1.6)

Two more options, off by default (the output above is unchanged byte for
byte without them):

- `--untitled-every N`: every Nth text document (files N−1, 2N−1, …) has no
  title in its metadata. PDFs drop Title and Author from the Info dictionary
  and print the title in 24 pt at the top of page 1 instead, as papers do;
  EPUBs leave out `dc:title`. Used by P1-18 (`tests/metadata.rs`).
- `--copies N`: N byte-identical copies of spread-out files, each in
  `copies/copy-NNNNN/`, so Library › Duplicates has many groups. The search
  benchmark uses `--copies 2000` at 30k and `--copies 100` at 1k.

```sh
node scripts/generate-fixtures.mjs ~/atlas-fixtures/titles --files 100 --text-kb 4 --untitled-every 4 --copies 20
```

`src-tauri/tests/pdfium.rs` checks that the bundled PDFium reads every page
and marker of a generated text fixture. The extraction worker's built-in
self-test PDF is written by the same code (`selfTestPdf` in
`scripts/lib/fixture-text.mjs`).

## Layout

`--flat N` adds N files in one folder on top of `--files`. The benchmark
suite (`just bench`, [docs/benchmarks.md](../docs/benchmarks.md)) and the
smoke test (`just e2e`) generate their own copies in a temporary folder.
The 30k set is a permanent benchmark asset (phase0-spec §11): body text
(`--text-kb`) is added without changing its layout.

The one exception to "never commit real books or papers" is the Slice 0.9
**spike corpus** in [`spikes/extraction-bench/corpus/`](../spikes/extraction-bench/corpus/):
a few MiB of public-domain and CC BY 4.0 documents (Project Gutenberg,
arXiv, the Federal Register, an 1894 scan) plus generated PDFs, each listed
with its source, license and SHA-256 in `manifest.json`. It measures text
extraction; `just spike-corpus` verifies it. See [spikes/README.md](../spikes/README.md).

The output folder must be empty or new, outside this repository, and not
inside an Atlas data root; the generator refuses otherwise. It prints a JSON
summary including `catalogableFiles`, the number of PDF/EPUB files Atlas
should catalog.

Each fixture includes:

- PDFs and EPUBs (stubs, or with `--text-kb` real documents) across nested folders up to five levels deep, with
  mixed-case extensions (`.PDF`, `.EPUB`)
- names with Unicode, CJK, double and leading spaces, punctuation, and a
  120-character name
- the same file name in several folders
- unsupported look-alikes (`archive.epub.zip`, `paper.pdf.part`, `.txt`,
  `.png`, an extensionless file)
- empty folders
- a symlink loop back to the root and a symlink to a folder outside it
  (`<dir>-outside`), where the platform allows symlinks
- an unreadable folder `locked/` on macOS and Linux

The locked folder must be made readable before deleting the fixture; the
generator prints the exact command.

## Search evaluation questions

[`evaluation/questions.json`](evaluation/questions.json) is the committed
retrieval evaluation for Phase 1 search (phase1-spec §10): 40 questions
over the spike corpus's text documents, a third each of exact wording
(typed without quotes), quoted phrases, and paraphrases in the user's own
words. They were written from the corpus truth texts before search was
built and are not edited to fit it; `src-tauri/tests/search_eval.rs` asks
each one and requires a relevant document in the top 5 for at least 80% of
them.

The format, also used for personal question sets with `just eval`:

```json
{
  "version": 1,
  "questions": [
    { "id": "q01", "kind": "exact", "question": "Quoth the Raven Nevermore", "relevant": ["epub-pg1065"] }
  ]
}
```

`relevant` lists document ids from `spikes/extraction-bench/corpus/manifest.json`
(for personal sets, file paths relative to a library folder). `kind` is
`exact`, `phrase`, `paraphrase` or `vague`.

[`evaluation/questions-vague.json`](evaluation/questions-vague.json) is the
Phase 2 set (phase2-spec §10): 40 questions in a reader's own words that
avoid each document's distinctive wording, so full-text search alone should
miss most of them. Written on 2026-11-09 before any semantic search was
built and not edited to fit it.

[`evaluation/questions-off.json`](evaluation/questions-off.json) holds 12
questions the corpus has nothing on (`kind: "off"`, `relevant: []`). With
the vague set, `just eval-ask` uses them to tune Ask Atlas's weak-retrieval
margin and to check that an answer says the library doesn't cover them.
Three describe well-known works the corpus lacks (a whaling captain, a
Baker Street detective, a summer's-day sonnet). Written on 2026-11-10,
before the run that uses them.

`scripts/evaluation-questions.test.mjs` keeps the sets well formed, every
relevant id a readable corpus document, and every off question without
one.

## The resume scenario (`knowledge/resume-scenario.json`)

The script for P3-13 (phase3-spec §10), which `src-tauri/tests/resume.rs`
runs on every OS:

- **The library:** generated books without copies, and the spike corpus's
  `pg11.epub`.
- **The session:** an earlier session; three searches, two questions and
  four opens; what the user kept.
- **Twelve weeks later:** a file moved, one renamed, one deleted, a book
  read again, and the embedding model switched.
- **The expected states** of every source opened.

Files are named as the catalog shows them, and searches find them by their
marker words.
