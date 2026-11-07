# Atlas

Atlas is a local-first personal AI librarian for large private collections of
PDFs, EPUBs, books and papers. *Collect freely. Organize minimally. Retrieve
intelligently.*

**Status: Phase 0, Slice 0.2.** Atlas launches, asks where to keep its
library, creates that data folder safely, and reopens it on every launch. It
remembers your theme, and it explains and recovers when the library's drive
is unplugged or another Atlas has it open. It does not yet read, catalog, or
open any of your documents, and it makes no network requests.

| Capability | Status |
| --- | --- |
| App shell, navigation, light/dark/system theme | Working |
| Atlas data location, settings persistence | Working |
| Library folders | Slice 0.3 |
| Device check, low-space and battery policy | Slice 0.4 |
| Background cataloging (pause, resume, recovery) | Slice 0.5 |
| Browse, filename search, open and reveal originals | Slice 0.6 |
| AI endpoint connection test | Slice 0.7 |
| Packaging and cross-platform evidence | Slice 0.8 |
| Extraction and vector-engine decision | Slice 0.9 |

Specifications: [product](docs/product-spec.md), [Phase 0](docs/phase0-spec.md),
[architecture](docs/architecture.md), [ADRs](docs/adr/).

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
```

Run `just` with no arguments to list every recipe.

## Build

```sh
just bundle           # debug bundles for this OS under src-tauri/target/debug/bundle/
```

Bundles are unsigned and not notarized. macOS Gatekeeper and Windows
SmartScreen will warn when opening them. See
[platform validation](docs/platform-validation.md) for what has and has not
been tested on each OS.

## Repository layout

```text
src/                  Svelte 5 + TypeScript UI
  lib/api/            typed IPC facade; bindings/ is generated from Rust (do not edit)
  lib/stores/         view and theme state
  lib/components/     shell, navigation, status, notices
  views/              Welcome, Home, Library, Settings
src-tauri/            Rust core and Tauri configuration
  src/commands/       thin IPC adapters
  src/domain/         DTOs and service traits
  src/services/       one module per responsibility (stubs in Slice 0.1)
  tests/              Rust integration tests
docs/                 specifications, architecture, ADRs, licenses, platform evidence
scripts/              license report, bindings check
```

## License

Not yet chosen; this is an open decision in the product specification.
Dependency licenses are inventoried in
[docs/dependency-licenses.md](docs/dependency-licenses.md).
