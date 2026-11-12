# Atlas

Atlas is a local-first personal AI librarian for large private collections of
PDFs, EPUBs, books and papers. *Collect freely. Organize minimally. Retrieve
intelligently.*

**Status: Phase 3, Slice 3.8 (export and import, after the PDF page view, interests, projects and rediscovery, bookmarks and saved findings, the Trails screen, recording trails, the knowledge store and the carry-overs); Phase 2 complete (version 0.3.0, [close-out review](docs/phase2-closeout.md)); Phases 0 and 1 complete ([Phase 1 review](docs/phase1-closeout.md)).** Atlas asks where to keep its library and
which folders hold your books and papers, then catalogs every PDF and EPUB
in them in the background, read-only. Cataloging slows on battery, pauses
when the battery is low or the drive is nearly full, survives quitting and
crashes, and resumes where it left off. Files offloaded to iCloud or
OneDrive are listed without being downloaded. The Library lets you search
file and folder names (ignoring case and accents), filter, sort, and open
files or show them in Finder or Explorer, even while cataloging continues.
Atlas then reads the text of each file in a separate
worker process: the Library shows each file's Text status (ready, needs
OCR, encrypted, only in the cloud, couldn't read, with the reason),
identical copies are read once, and reading slows on battery and pauses at
20%, like cataloging. You can search that text from Home or the Search
tab: words, "exact phrases" in quotes, and prefixes ending in `*`, ignoring
case and accents. Results are grouped by book or paper, with the page or
chapter and a snippet that highlights your words; when no result has all of
them, Atlas shows the ones with some, and says so. Selecting a file opens
its details: title, authors, language, pages, text status and the other
places the same content is stored. When a file's own title is missing or
junk ("Untitled", "Microsoft Word - draft3.docx"), Atlas takes the
largest text on a PDF's first page, or tidies the file name, and says
which. Library › Duplicates lists identical copies, most wasted space
first (Atlas never deletes or moves them), and Settings › Storage shows
what the catalog, the text and the search index take, estimates what the
rest of the text will need, and cleans caches. Resuming a pause inside a
folder of 20,000 files picks up again in about a second, and if Atlas is
force-quit or crashes, its text reader stops with it.
In Settings › AI Models you can connect Ollama, oMLX, LM Studio, or another
OpenAI-compatible service and test it; keys are stored in your system
keychain. You then choose which model makes embeddings and which one chats:
Atlas checks each with one short fixed request, reports the embedding
model's dimensions, and, if nothing suitable is installed, recommends a
model for your computer and your AI app with the steps to get it (it never
downloads one). Each service is labelled as this computer, another
computer, or an internet service. Atlas contacts a service only when you
test it, and sends nothing from your library. The AI features that use
these models arrive in the next Phase 2 slices. Once you choose an
embedding model, Atlas indexes the meaning of your library in the
background: it sends the text it has read, chunk by chunk, to that model,
never twice, pauses on battery, waits when the AI service is busy or down,
and keeps the vectors in its catalog with a vector index that survives
crashes. A status line shows its progress, Settings › Storage shows the
space it takes, and an internet service is used only if you allow it in
Settings › Privacy. Search then uses meaning as well as words: a question
in your own words finds the book it describes even when the book never
uses those words, marked "Similar meaning". A document's details list the
other documents in your library closest to it in meaning (never its own
copies), and More like this on a result opens them; this compares only
what Atlas has stored, so it works while the AI service is down. With a
chat model chosen, Atlas can answer a question from the passages it finds
in your library, citing them by number. Every citation is checked against
the passages it sent, and invented ones are removed. An internet service
gets nothing until you have seen what will be sent and agreed, and
Settings › Privacy can turn all external AI off. The Ask Atlas screen
streams the answer with citation chips and lists the sources used. Since
Phase 3, files downloaded from iCloud are read at the next rescan, and
Project Gutenberg's header and licence are left out of the books' text.
Atlas now keeps a separate store for what you will save (trails, bookmarks,
notes), takes a snapshot of it once a day when it has changed, and never
lets a damaged or newer store stop your library from opening. Settings ›
Storage shows it as "Your trails and notes" and "Snapshots". Atlas now
records a trail of what you search for, what you ask and what it answered,
and the sources you open from them, on this computer only. The status bar
says "Trail: recording" and pauses it with one click, Search has New
trail, and Settings › Privacy can stop recording, keep unpinned trails
past 90 days, or delete them all. The Trails view shows every trail as a
timeline: what you searched, what you asked and what Atlas answered then,
and what you opened, each source saying whether it is still in your
library. Continue, Search again and Ask again pick a trail up, and Home
lists the three most recent. You can bookmark a place (a result, a cited
passage, a document) and save an answer or a passage as a finding, each
with a note of your own; the Saved tab lists them, and the details panel
shows a document's bookmarks. Interests and projects group trails,
bookmarks, findings and documents (Add to… wherever they are shown), and
Home suggests documents from your library near them and your recent trails,
from the vectors Atlas already has: no model is asked.

Every push to `main`, every release tag and every pull request is built
and tested on macOS (Apple silicon), Windows x64, and Linux x64; pushes to
work-in-progress slice branches get a quicker Linux-only check
([CI tiers](docs/architecture.md#build-ci-and-measurement)). CI runs an automated click-through of the app on Windows and
Linux, measures cataloging performance on all three, and produces
unsigned development installers. See [packaging](docs/packaging.md) and
[benchmarks](docs/benchmarks.md). Plugged in, cataloging is throttled to 500
folder entries a second (30,000 files in 80–100 s on the tested machines),
using about 7% of one core on an M5 Pro and around 20 MiB of memory. Browsing
stays responsive meanwhile: under 4 ms at the 95th percentile with 1,000
files and under 130 ms with 30,000.

Phase 1 reads the documents' text in the background and makes every
book and paper searchable by what it says, without AI. Its scope, slices,
required tests and acceptance checklist are in the
[Phase 1 specification](docs/phase1-spec.md). A measured spike
([results](docs/spike-0.9.md), [ADR 0008](docs/adr/0008-extraction-and-vector-engines.md))
chose the libraries for that and for semantic search later:

- **PDFium** for PDFs, running in a helper process;
- **rbook** for EPUBs;
- **usearch** for vector search.

Docling was measured as a bundled alternative, and wasn't bundled: about 1 GB
compressed, 3.8 GB of memory, and no better reading order than PDFium.
PDFium and rbook have read text in the app since Phase 1; usearch arrives
with semantic search.

| Capability | Status |
| --- | --- |
| App shell, navigation, light/dark/system theme | Working |
| Atlas data location, settings persistence | Working |
| Library folders | Working |
| Device check, low-space and battery policy | Working |
| Background cataloging (pause, resume, recovery) | Working |
| Browse, filename search, open and reveal originals | Working |
| AI endpoint connection test | Working |
| CI, installers, cross-platform smoke test, benchmarks | Working (development builds, unsigned) |
| Extraction and vector-engine decision | Decided ([ADR 0008](docs/adr/0008-extraction-and-vector-engines.md)); built in Phase 1 and 2 |
| Background jobs of several kinds (scans now, text extraction next) | Working (Phase 1, Slice 1.1) |
| Reading text from PDFs and EPUBs, in a separate worker process; per-file Text status in the Library | Working (Slice 1.3): slows on battery, pauses at 20%; copies and moved files are read once; scans, DRM and damaged files get a status and a reason |
| Search the text of your books and papers: Search tab and Home, phrases, prefixes, accent-insensitive, highlighted snippets | Working (Slices 1.4–1.5) |
| Document details: title, authors, language, pages, text status, other copies (Also at) | Working (Slice 1.5) |
| Titles from metadata, a PDF's first page or the file name; junk metadata cleaned | Working (Slice 1.6) |
| Duplicates view: identical copies grouped, most extra space first | Working (Slice 1.6) |
| Settings › Storage: usage by category, indexing estimate, Clean caches | Working (Slice 1.6) |
| Phase 1 carry-overs: fast resume inside large folders, Windows junctions and paths over 260 characters, no text reader outliving the app, extraction benchmark, a smaller catalog | Working (Slice 1.7) |
| Phase 1 close-out: evidence on three OSes, version 0.2.0 | Done ([review](docs/phase1-closeout.md)); Windows and Linux verified in CI, with documented gaps |
| Choosing the chat and embedding models, with a capability test and recommendations | Working (Phase 2, Slice 2.1) |
| Chunks, embeddings in the catalog, and a crash-safe vector index | Working (Phase 2, Slice 2.2) |
| Indexing meaning in the background: status, battery pause, backoff, model change, Storage, privacy opt-in | Working (Phase 2, Slice 2.3) |
| Search by meaning as well as words: "Similar meaning" results, fused with text matches; Phase 1's results whenever meaning search can't run | Working (Phase 2, Slice 2.4) |
| Related documents in the details panel, and More like this on a result | Working (Phase 2, Slice 2.5) |
| Ask Atlas engine: answers from your library through your chat model, citations checked as they stream, prompt-injection bounds; Settings › Privacy: Disable all external AI, agreed services | Working (Phase 2, Slice 2.6; the screen is Slice 2.7) |
| Ask Atlas screen: Search's Find sources \| Ask Atlas switch, the streamed answer with citation chips that open the passage, Sources used, Stop, the disclosure for internet services | Working (Phase 2, Slice 2.7) |
| Phase 2 close-out: evidence on three OSes, version 0.3.0 | Done ([review](docs/phase2-closeout.md)); Windows and Linux verified in CI with a scripted model service, with documented gaps |
| Phase 3 carry-overs: downloaded iCloud files read at the next rescan, a weak-retrieval margin per embedding model, Project Gutenberg headers and licences left out when a book is read | Working (Phase 3, Slice 3.1) |
| Phase 3, the knowledge store: your trails and notes in a database of their own (`library/knowledge.sqlite`), never blocking the library, snapshotted daily when changed (14 kept), shown in Settings › Storage and never touched by Clean caches | Working (Phase 3, Slice 3.2) |
| Phase 3, recording trails: submitted searches, questions with their answers and sources, and the sources you open from them, in 30-minute sessions; pause from the status bar, New trail, 90-day expiry of unpinned trails, Delete all trails in Settings › Privacy; nothing leaves the knowledge store | Working (Phase 3, Slice 3.3) |
| Phase 3, the Trails screen: every trail, its timeline with the answers as recorded and each source's state now, Continue, Search again and Ask again; Home's "Pick up where you left off" | Working (Phase 3, Slice 3.4) |
| Phase 3, bookmarks and saved findings: Bookmark on results, details, cited passages and sources; Save as a finding on answers and passages; notes; the Saved tab; Bookmarks in this document | Working (Phase 3, Slice 3.5) |
| Phase 3, interests and projects: Add to… on trails, saved items and documents; each collection's page; finishing and archiving; suggestions from your library on Home, with Not for this | Working (Phase 3, Slice 3.6) |
| Phase 3, the page view: Show the page on PDF citations, results, bookmarks and trail steps; the page rendered in a separate, killable worker with the passage outlined when found; Previous, Next, a page number, Fit width or Actual size, Bookmark this page; the page's text below for screen readers and copying; pages cached by content (at most 200 MB) | Working (Phase 3, Slice 3.7) |
| Phase 3, export and import: Export… on a trail, a finding, a collection or everything, as Markdown notes (Obsidian or Markdown links) or one JSON file, into an `Atlas` folder inside a folder you choose; your own notes there are never overwritten; Import from a file… with a dry run first, the newer copy kept, nothing removed | Working (Phase 3, Slice 3.8) |
| Phase 3, the knowledge workspace: research trails, bookmarks and saved findings, interests and projects, a PDF page view, Markdown and JSON export | Phase 3, in progress ([spec](docs/phase3-spec.md)): Slices 3.1 to 3.8 of 3.9 built |

Specifications: [product](docs/product-spec.md), [Phase 0](docs/phase0-spec.md),
[Phase 1](docs/phase1-spec.md), [Phase 2](docs/phase2-spec.md),
[Phase 3](docs/phase3-spec.md),
[architecture](docs/architecture.md),
[ADRs](docs/adr/). Evidence:
[Phase 0 close-out review](docs/phase0-closeout.md),
[Phase 1 close-out review](docs/phase1-closeout.md),
[Phase 2 close-out review](docs/phase2-closeout.md),
[platform validation](docs/platform-validation.md),
[benchmarks](docs/benchmarks.md), [packaging](docs/packaging.md),
[extraction and vector spike](docs/spike-0.9.md),
[dependency licenses](docs/dependency-licenses.md)
([spike candidates](docs/spike-dependency-licenses.md)).

## Install a development build

Download the installer for your OS from the artifacts of a green
[CI run](https://github.com/markcheno/Atlas/actions/workflows/ci.yml)
(`atlas-<OS>-…-release`). The builds are unsigned, so macOS and Windows
will warn the first time. [Packaging](docs/packaging.md) explains how to
open them and lists the Linux runtime dependencies.

## Prerequisites

End users need nothing beyond the installer. Developers need:

- Rust via [rustup](https://rustup.rs). The pinned toolchain in
  `rust-toolchain.toml` (1.95.0, with rustfmt and clippy) installs on first use.
- Node.js 22 or newer (CI uses 24) and pnpm 11 (`corepack enable` picks up the
  version pinned in `package.json`).
- The pinned PDFium library, which every Rust build needs because the app
  bundles it: `just setup` (or `just fetch-pdfium`, or
  `node scripts/fetch-pdfium.mjs`) downloads it from bblanchon/pdfium-binaries,
  checks its SHA-256 against `tools.lock.json`, and unpacks it into
  `src-tauri/vendor/pdfium/` (gitignored). It is pinned for macOS on Apple
  silicon, Windows x64 and Linux x64, the platforms Atlas builds on.
- Platform build tools:
  - **macOS:** Xcode Command Line Tools (`xcode-select --install`).
  - **Windows:** Visual Studio Build Tools with "Desktop development with C++",
    and the WebView2 runtime (preinstalled on Windows 10 21H2+ and 11).
  - **Linux (Debian/Ubuntu):**
    `sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev patchelf file build-essential`.
    Other distributions: see the
    [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

Optional: [just](https://just.systems) (`brew install just`, `winget install Casey.Just`,
or `cargo install just`) for the shorthand commands below. Every recipe wraps a
pnpm script, so pnpm alone also works.

## Run

```sh
just setup    # or: rustup toolchain install && pnpm install --frozen-lockfile && node scripts/fetch-pdfium.mjs
just dev      # or: pnpm tauri dev
just dev-simulate battery:18   # debug build with simulated power (ac, battery:<pct>, unknown, desktop)
just dev-unthrottled           # debug build without the cataloging rate limit or the reading duty cycle
just fixtures ~/atlas-fx 2000  # synthetic test library (see fixtures/README.md)
just fixtures ~/atlas-fx30k 10000 20000   # the 30k benchmark set (with a 20k-file folder)
just fixtures ~/atlas-fxtext 1000 0 20    # with about 20 KiB of body text per file
```

`just ui` (`pnpm dev`) serves only the UI in a browser for layout work; native
commands then report that they are unavailable outside the desktop app.

## Check

```sh
just check            # rustfmt, clippy -D warnings, cargo test, eslint, prettier, svelte-check, vitest
just ci               # check plus the bindings drift check, as CI runs it
just bindings         # regenerate TypeScript bindings from Rust DTOs
just fmt              # apply prettier and rustfmt
just licenses         # regenerate docs/dependency-licenses.md
just clean            # remove the Rust build outputs and dist/ (rebuilt on demand)
just clean-all        # also the spikes' build outputs: everything a build makes again
just bench            # benchmark suite on 1k and 30k fixtures (release build; about 25 minutes)
just bench-full       # plus the 30k throttle sweep
just bench-search     # search benchmark on the 30k text fixture (fails above 250 ms p95)
just bench-extraction # extraction benchmark on the 30k text fixture (`just bench-extraction 1k` adds battery)
just bench-reader     # page view benchmark: corpus PDF pages rendered at 1,200 px, a cache hit, the worker's memory
just bench-knowledge  # knowledge store at 100,000 steps: imported, the Trails list, recording, a snapshot, export
just generate-knowledge out.json   # a generated store as a JSON export, to try Import with
just eval questions.json ~/AtlasData   # ask a question set against a library, read-only
just eval-ask         # Ask Atlas against your own embedding and chat models on the corpus (owner-run)
just e2e              # WebDriver smoke test of the debug build (Windows/Linux; needs tauri-driver)
just fake-ai          # the smoke test's scripted model service on 127.0.0.1:8765, to try Ask Atlas without a model
just worker-hello     # start the extraction worker and check it loads PDFium (time to hello)
just ci-remote        # run CI on GitHub for this pushed branch: full (default) or `just ci-remote quick`
just ci-status        # recent CI runs for this branch
```

The Slice 0.9 spike has its own commands (they need [uv](https://docs.astral.sh/uv/);
see [spikes/README.md](spikes/README.md)):

```sh
just spike-tools      # pinned PDFium, protoc and typst into spikes/.tools
just spike-corpus     # verify the spike corpus (checksums, licenses)
just spike-extract    # PDF/EPUB extraction comparison
just spike-docling    # freeze the Docling sidecar (~2 GB download)
just spike-vectors    # vector engines at 100k (pass 1000000 for 1M)
just spike-builds     # clean build time and size per candidate
just spike-licenses   # docs/spike-dependency-licenses.md
```

Run `just` with no arguments to list every recipe.

## Build

```sh
just bundle           # debug bundles for this OS under src-tauri/target/debug/bundle/
just bundle-release   # release bundles under src-tauri/target/release/bundle/, with sizes
```

Bundles are unsigned and not notarized. macOS Gatekeeper and Windows
SmartScreen will warn when opening them ([packaging](docs/packaging.md)).
See [platform validation](docs/platform-validation.md) for what has and has
not been tested on each OS.

## Repository layout

```text
src/                  Svelte 5 + TypeScript UI
  lib/api/            typed IPC facade; bindings/ is generated from Rust (do not edit)
  lib/stores/         app, workspace, sources, inventory, extraction, library, duplicates, search, details, originals, provider, theme and view state
  lib/components/     shell, navigation, status bar, panels, notices, document details, Open/Reveal actions
  views/              onboarding, Home, Search, Library, Settings
src-tauri/            Rust core and Tauri configuration
  src/commands/       thin IPC adapters
  src/domain/         DTOs and service traits
  src/services/       catalog, inventory, extraction, search, documents, storage, library, sources, providers, workspace, …
  src/infrastructure/ OS, filesystem, HTTP and keychain adapters
  migrations/         catalog schema (SQLite), applied in order
  tests/              Rust integration tests
  benches/            benchmark harness (docs/benchmarks.md)
e2e/                  WebDriver smoke test (Windows, Linux)
fixtures/             how to generate synthetic test libraries
docs/                 specifications, architecture, ADRs, evidence, licenses
scripts/              fixtures, benchmarks, bundle sizes, license report, bindings check
spikes/               Slice 0.9 extraction and vector spike (not part of the app)
.github/              CI workflow and shared setup action
```

## License

Not yet chosen; this is an open decision in the product specification.
Dependency licenses are inventoried in
[docs/dependency-licenses.md](docs/dependency-licenses.md).
