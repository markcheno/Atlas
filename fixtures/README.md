# Test fixtures

Atlas never ships or commits real books or papers. Test libraries are
generated on demand from tiny synthetic stubs:

```sh
just fixtures ~/atlas-fixtures/small                 # 1,000 PDF/EPUB stubs (1,006 catalogable)
just fixtures ~/atlas-fixtures/large 10000 20000     # the 30k benchmark set: 10,000 nested files
                                                     # plus one flat folder of 20,000 (30,006 catalogable)
```

`--flat N` adds N files in one folder on top of `--files`. The benchmark
suite (`just bench`, [docs/benchmarks.md](../docs/benchmarks.md)) and the
smoke test (`just e2e`) generate their own copies in a temporary folder.
The 30k set is a permanent benchmark asset (phase0-spec §11): later phases
add synthetic body text and chunk counts without changing its layout.

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

- PDF and EPUB stubs across nested folders up to five levels deep, with
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
