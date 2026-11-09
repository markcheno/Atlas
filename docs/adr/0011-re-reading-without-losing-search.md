# ADR 0011: Re-reading without losing search

- **Status:** Accepted (Phase 1, Slice 1.6)
- **Date:** 2026-11-09
- **Context documents:** [Phase 1 spec](../phase1-spec.md) §6.1, P1-18;
  [ADR 0009](0009-jobs-and-extraction.md) (documents and the extraction
  job); [ADR 0010](0010-full-text-search.md) (passages and the index)

## Context

Slice 1.3 records which reader version produced a document, and reads it
again when the reader changes. As built, an older version **unlinked** the
document's files on open (`document_id = NULL`, `pending`): until each file
was read again it showed "Waiting" and dropped out of search. On a library
of tens of thousands of books on battery that is hours with search results
missing, for changes that often do not touch the text at all.

Slice 1.6 changes two readers:

- EPUB text is written without the Markdown marks html2text's plain
  decorator adds (`# Chapter 1`, `[link]`, `* `), which showed in snippets.
- PDFs gain a title read from the first page, for files whose metadata has
  no usable title.

## Decision

**Each format has its own reader version** (`READER_VERSION_PDF`,
`READER_VERSION_EPUB`). Changing one reader re-reads only documents of
that type.

**On open, a document read by an older reader is either moved to the
current version or marked to be read again (`documents.refresh`, migration
0010).** It is moved when re-reading could not change it: a document
without text keeps its reason, and a PDF whose metadata already gives a
usable title gains nothing from the first-page title. Everything else is
marked.

**A marked document stays linked, searchable and "Text ready".** The
extraction job reads the waiting files first, then one local copy of each
marked document. The new passages replace the old ones in the same
single transaction as any document (ADR 0009), so search sees the old text
or the new, never neither. Copies are read once. A document the worker
crashes on twice keeps its old text and is no longer marked. The status
line says "Updating text · N files to go"; the counts by status do not
change.

## Consequences

- Upgrading a library never empties search. After Slice 1.6, every EPUB
  and every PDF without a usable metadata title is read again once, in the
  background, under the usual battery and space policy.
- The cleaning of titles and authors happens when the catalog is read, not
  when it is written, so changing those rules needs no re-read at all.
- Orphaned documents (no files left) are not re-read.
- The old unlink-and-re-read path no longer runs for version changes; a
  file whose bytes changed is still read as new content, as before.
