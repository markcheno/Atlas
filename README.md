# Atlas

Atlas is a local-first personal AI librarian for large private collections of
PDFs, EPUBs, books and papers. *Collect freely. Organize minimally. Retrieve
intelligently.*

**Status: Phase 0, Slice 0.9 (the last Phase 0 slice).** Atlas asks where to keep its library and
which folders hold your books and papers, then catalogs every PDF and EPUB
in them in the background, read-only. Cataloging slows on battery, pauses
when the battery is low or the drive is nearly full, survives quitting and
crashes, and resumes where it left off. Files offloaded to iCloud or
OneDrive are listed without being downloaded. The Library lets you search
file and folder names (ignoring case and accents), filter, sort, and open
files or show them in Finder or Explorer, even while cataloging continues.
In Settings › AI Models you can connect Ollama, oMLX, LM Studio, or another
OpenAI-compatible service and test it; keys are stored in your system
keychain. Atlas contacts a service only when you test it, and sends nothing
from your library. AI features themselves come in a later phase.

Every push is built and tested on macOS (Apple silicon), Windows x64, and
Linux x64. CI runs an automated click-through of the app on Windows and
Linux, measures cataloging performance on all three, and produces
unsigned development installers. See [packaging](docs/packaging.md) and
[benchmarks](docs/benchmarks.md). Plugged in, cataloging is throttled to 500
folder entries a second (30,000 files in 80–100 s on the tested machines),
using about 7% of one core on an M5 Pro and around 20 MiB of memory. Browsing
stays responsive meanwhile: under 4 ms at the 95th percentile with 1,000
files and under 130 ms with 30,000.

Phase 1 will read the documents' text. A measured spike
([results](docs/spike-0.9.md), [ADR 0008](docs/adr/0008-extraction-and-vector-engines.md))
chose the libraries for that and for semantic search later:
- **PDFium** for PDFs, running in a helper process;
- **rbook** for EPUBs;
- **usearch** for vector search.

Docling was measured as a bundled alternative, and wasn't bundled: about 1 GB
compressed, 3.8 GB of memory, and no better reading order than PDFium.
None of this is in the app yet.

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

Specifications: [product](docs/product-spec.md), [Phase 0](docs/phase0-spec.md),
[architecture](docs/architecture.md), [ADRs](docs/adr/). Evidence:
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
just setup    # or: rustup toolchain install && pnpm install --frozen-lockfile
just dev      # or: pnpm tauri dev
just dev-simulate battery:18   # debug build with simulated power (ac, battery:<pct>, unknown, desktop)
just dev-unthrottled           # debug build without the cataloging rate limit
just fixtures ~/atlas-fx 2000  # synthetic test library (see fixtures/README.md)
just fixtures ~/atlas-fx30k 10000 20000   # the 30k benchmark set (with a 20k-file folder)
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
just bench            # benchmark suite on 1k and 30k fixtures (release build; a few minutes)
just bench-full       # plus the 30k throttle sweep
just e2e              # WebDriver smoke test of the debug build (Windows/Linux; needs tauri-driver)
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
  lib/stores/         app, workspace, sources, inventory, library, provider, theme and view state
  lib/components/     shell, navigation, status bar, panels, notices
  views/              onboarding, Home, Library, Settings
src-tauri/            Rust core and Tauri configuration
  src/commands/       thin IPC adapters
  src/domain/         DTOs and service traits
  src/services/       catalog, inventory, library, sources, providers, workspace, …
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
