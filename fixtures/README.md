# Test fixtures

Atlas never ships or commits real books or papers. Test libraries are
generated on demand from tiny synthetic stubs:

```sh
just fixtures ~/atlas-fixtures/small            # 1,000 PDF/EPUB stubs
just fixtures ~/atlas-fixtures/large 30000      # benchmark size
node scripts/generate-fixtures.mjs <dir> --files 2000 --flat 20000   # plus one 20,000-entry folder
```

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
