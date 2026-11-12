# ADR 0018: The PDF page view, and a second worker to render it

- **Status:** Accepted (Phase 3, Slice 3.7).
- **Date:** 2026-11-11
- **Context documents:** [Phase 3 spec](../phase3-spec.md) §5.5, §7.5, §12
  (P3-19 to P3-21), §13; [ADR 0008](0008-extraction-and-vector-engines.md)
  (PDFium); [ADR 0009](0009-jobs-and-extraction.md) (the extraction
  worker); [ADR 0016](0016-the-knowledge-store.md) (anchors)

## Context

A citation, a bookmark or a search result in a PDF shows only its
passage's text. Readers want to see the page itself: the figure beside
the sentence, the footnote, the layout the author chose. Phase 3 adds a
page view for PDFs. EPUBs have no fixed pages and keep the passage text.

Four facts shaped it:

- **The main process never loads PDFium.** That has been true since
  Phase 1 (ADR 0009). A bad PDF can crash or hang PDFium, and only a
  separate process can be killed and restarted safely.
- **The extraction worker is busy for hours** on a new library. A page
  asked for while it reads must not wait behind a 300-page book, and a
  stuck render must not stop reading.
- **Rendering is fast but its output is large.** A 1,200 px page is
  200 KB to 1 MB of PNG. The worker protocol is newline JSON with no frame
  limit, so the image needs a bound.
- **The renderer is not trusted with paths** (ADR 0015, ADR 0017). It
  sends ids and text it already shows, and nothing else.

## Decision

### A second worker, for rendering only

- `services/reader/` owns its own `Supervisor` over the same executable
  (`WorkerCommand::current_exe`), with its own stop flag and statistics.
- It starts at the first render and is killed after 60 seconds idle, or
  when the library closes (the `ReaderService` is part of `OpenRoot`).
- One render at a time, behind a mutex. Extraction never sends to it, and
  the reader never sends to the extraction worker, so neither waits for the
  other. Two PDFium processes run only while both are working.

### Protocol version 2

`PROTOCOL_VERSION` goes from 1 to 2 and adds:

- `Request::Render { path, page, width, find? }`: the page counts from 1,
  the width is in pixels, and `find` is optional text to look for.
- `Frame::Rendered { page, pageCount, width, height, png, rects, found }`:
  the PNG in base64, and the rectangles of the found text in the image's
  pixels.
- `Frame::RenderFailed { reason }`, where the reason is one of
  `page_out_of_range`, `encrypted`, `malformed`, `too_large`, `cloud_only`
  or `bad_path`. A refusal is an answer: the worker keeps running. The main
  process adds its own reasons to the `render_failed` error: `not_read` (no
  catalog entry yet), `not_a_pdf`, `unavailable` (no worker configured)
  and `worker` (the worker was killed or crashed).

`Supervisor::render` is a typed method beside `hash` and `extract`. A
`Rendered` frame whose base64 won't decode, or any other frame, is a
protocol fault, and the supervisor kills the worker.

### Limits

- **The width:** 1 to 2,000 px, checked in the main process before
  anything else (`invalid_render`).
- **The long edge:** at most 3,000 px. A page too tall at the width asked
  for is scaled down to fit, not refused, so a long page still shows.
- **The PNG:** at most 8 MiB, or `too_large`.
- **Time and memory:** the extraction worker's own limits, which are 30
  seconds without a frame and the memory cap. Over either, the worker is
  killed, the view says it couldn't show the page, and the next render
  starts a new worker.

### Finding the passage

The text to find has its whitespace collapsed. PDFium's text search looks
for its first 200 characters on the page, and if they aren't found, for the
first 60.

**Amended in Slice 3.9**, after the release DMG smoke. If neither start is
found, the search tries the text in pieces of about 60 characters (at most
16). Pieces break at what PDFium matches differently: the mark it puts on a
word hyphenated at a line end, other control characters, a snippet's
ellipses, and private-use characters. Every piece that is on the page is
outlined. The benchmark's corpus pages went from 33 to 37 of 37 outlined.

Each match's segments become rectangles through `points_to_pixels`. Not found means no rectangles and `found: false`, never
an error: a scanned page, or text PDFium reads differently, still shows.

`find` is text the screen already shows (an excerpt, a snippet). The
command takes at most 2,000 characters of it. It is searched for and used
in the cache key's hash, and never stored.

### The outline is drawn by the app

The worker returns rectangles, not a marked image. The page view draws them
over the image, scaled to the size shown, in the theme's accent colour.
Since Slice 3.9 they are an SVG over the image, in the image's pixels.
Inline-styled boxes were never drawn in the app, because Tauri's nonce in
`style-src` makes WebKit block `style` attributes.
That means one cached page serves both themes, and the outline never
carries meaning alone: the page's text, from the catalog's passages, is
shown under the image for screen readers and copying.

### The cache

- `cache/previews/<content hash>/<page>-<width>-<find key>.png`, with a
  `.json` beside it holding the page count, the size and the rectangles.
  The find key is 8 hex characters of the BLAKE3 hash of the normalised
  text, or `none`. The outline depends on it, so it is part of the key
  (the spec's draft had `<page>-<width>.png`).
- The cache is read before any file access. A moved or copied file's pages
  are reused, and a page already cached shows even when the file is now
  only in the cloud: nothing is downloaded to show it.
- At most 200 MB. After each write the least recently used pages go first,
  judged by modification time, which a cache hit refreshes. Clean caches
  empties it, and Settings › Storage counts it with the caches.
- Files are written beside their final name and then renamed, so a reader
  never sees half a page.

### Checks before rendering are Open's

`source_open::path_for(root, file_id, allow_download)` reuses Open's
lookup and resolution: the folder must be reachable, the path a regular
file inside its root, and a file only in the cloud needs `allowDownload`.
The errors are Open's (`file_cloud_only`, `file_cloud_placeholder`,
`file_missing`, and the rest). The page view asks the same question Open
asks before downloading.

### IPC

`render_page(fileId, pageNo, widthPx, find?, allowDownload?) -> PageImage`.
The workspace lock is held only to look up the hash, the page's text and
the path. Rendering runs on a blocking thread, outside the lock, so the
library stays responsive. The image crosses as base64 and is shown through
a `data:` URL, which the CSP's `img-src 'self' data:` already allows. The
CSP does not change.

### No PDFium in the main process, checked

P3-20 lists the main process's loaded modules after rendering: on Linux
through `/proc/self/maps`, on macOS through `_dyld_image_count` and
`_dyld_get_image_name`. No module's name may contain `pdfium`. Windows has
no cheap module list without new dependencies, so the check is skipped
there. The same code path runs on every OS.

## Consequences

- A page shows in tens of milliseconds on the reference machine (about
  30 ms for the first, with the worker starting, and 17 ms p95 after:
  [benchmarks](../benchmarks.md#page-view-slice-37)), far inside the 500 ms
  target.
- A second process exists while the user looks at pages, using about
  60 MiB, and goes away a minute after the last one.
- Protocol 2 workers answer version 1 requests unchanged. A version-1
  worker never receives `Render`, because the two ship together in one
  executable.
- The page view is not a reader. It has no text selection on the image, no
  search within the document and no continuous scrolling. Those would need
  a different design.
- The outline is missed where PDFium's page text differs from the
  extracted text. Superscript marks in a paper's author list and hyphens
  PDFium marks with a control character both cause it. The page still
  shows, with the text below it.
