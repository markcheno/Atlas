# Atlas developer commands. Run `just` to list them.
#
# Recipes are thin wrappers over the package.json scripts so each command is
# defined in one place. When you add, rename, or remove a package script or
# a common workflow, update this file in the same change.

set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

manifest := "src-tauri/Cargo.toml"

# Bundle formats built by `just bundle` on this OS (matches CI).
default_bundles := if os() == "macos" { "app,dmg" } else if os() == "windows" { "nsis" } else { "appimage,deb" }

# List available recipes
default:
    @just --list --unsorted

# ---------------------------------------------------------------- setup

# Install the pinned Rust toolchain, locked npm dependencies, and the pinned PDFium (needed before any Rust build)
setup:
    rustup toolchain install
    pnpm install --frozen-lockfile
    node scripts/fetch-pdfium.mjs

# Download and verify the pinned PDFium (tools.lock.json) into src-tauri/vendor/pdfium/; kept when already verified
fetch-pdfium:
    node scripts/fetch-pdfium.mjs

# ------------------------------------------------------------------ run

# Launch the desktop app with hot reload
dev:
    pnpm tauri dev

# Serve only the UI in a browser (native commands report unavailable)
ui:
    pnpm dev

# Launch with no cataloging rate limit and no reading duty cycle, for benchmarks (debug builds only)
dev-unthrottled $ATLAS_INVENTORY_UNTHROTTLED="1" $ATLAS_EXTRACT_UNTHROTTLED="1":
    pnpm tauri dev

# Launch with simulated power (ac, battery:<pct>, unknown, desktop) and optional free bytes, e.g. `just dev-simulate battery:18`
dev-simulate $ATLAS_SIMULATE_POWER $ATLAS_SIMULATE_FREE_BYTES="":
    pnpm tauri dev

# -------------------------------------------------------------- checks

# Run every check: Rust fmt/clippy/tests, lint, typecheck, frontend tests
check:
    pnpm check

# Everything CI runs before packaging, including the bindings drift check
ci: check check-bindings

# rustfmt --check, clippy -D warnings, cargo test
check-rust:
    pnpm check:rust

# ESLint and Prettier --check
lint:
    pnpm lint

# svelte-check and tsc
typecheck:
    pnpm typecheck

# Frontend tests once
test:
    pnpm test

# Frontend tests in watch mode
test-watch:
    pnpm test:watch

# Rust tests only; extra args go to cargo test, e.g. `just test-rust --test ipc_contract`
test-rust *filter:
    cargo test --manifest-path {{ manifest }} --locked {{ filter }}

# Apply Prettier and rustfmt
fmt:
    pnpm format

# ------------------------------------------------------------- bindings

# Regenerate TypeScript bindings from Rust DTOs
bindings:
    cargo test --manifest-path {{ manifest }} --locked --lib export_bindings

# Fail if generated bindings differ from what is committed
check-bindings:
    pnpm check:bindings

# ------------------------------------------------------------ packaging

# Build debug bundles for this OS, e.g. `just bundle` or `just bundle app`
bundle bundles=default_bundles:
    pnpm tauri build --debug --bundles {{ bundles }}

# Build unsigned release bundles for this OS, as CI's package job does
bundle-release bundles=default_bundles:
    pnpm tauri build --bundles {{ bundles }}
    node scripts/bundle-sizes.mjs src-tauri/target/release

# Open the macOS debug app bundle built by `just bundle`
[macos]
open-bundle:
    open src-tauri/target/debug/bundle/macos/Atlas.app

# Generate a synthetic library outside the repo, e.g. `just fixtures ~/fx 2000`, the 30k set `just fixtures ~/fx30k 10000 20000`, with body text `just fixtures ~/fx 1000 0 20`, or with untitled files and copies `just fixtures ~/fx 100 0 4 4 20` (see fixtures/README.md)
fixtures dir files="1000" flat="0" text_kb="0" untitled_every="0" copies="0":
    node scripts/generate-fixtures.mjs {{ dir }} --files {{ files }} --flat {{ flat }} --text-kb {{ text_kb }} --untitled-every {{ untitled_every }} --copies {{ copies }}

# ------------------------------------------------- benchmarks and smoke

# Start the extraction worker 10 times and check it says hello, e.g. `just worker-hello` or `just worker-hello src-tauri/target/debug/bundle/macos/Atlas.app/Contents/MacOS/atlas`
worker-hello exe="src-tauri/target/debug/atlas":
    cargo build --manifest-path {{ manifest }} --locked
    node scripts/worker-hello.mjs {{ exe }}

# Benchmark suite (cataloging on the 1k and 30k fixtures, search and extraction on the 30k text fixture; release build); results in the temp dir unless `out` is given
bench out="":
    pnpm bench {{ if out == "" { "" } else { "--out " + out } }}

# Benchmark suite plus the 30k throttle sweep (about 10 more minutes)
bench-full out="":
    pnpm bench --sweep {{ if out == "" { "" } else { "--out " + out } }}

# Search benchmark only (30k text fixture, release build); fails above 250 ms p95 while reading (P1-19, reference machine)
bench-search out="":
    pnpm bench --search-only --max-p95-ms 250 {{ if out == "" { "" } else { "--out " + out } }}

# Extraction benchmark only (release build): `size` 30k (default; Run and unthrottled) or 1k (also the Slow duty cycle), plus a kill mid-read
bench-extraction size="30k" out="":
    pnpm bench --extraction-only --extraction-size {{ size }} {{ if out == "" { "" } else { "--out " + out } }}

# The embedding job, hybrid search while it runs (Phase 2, P2-18) and related documents after it (Slice 2.5): `size` is 30k (the reference measurement, with a 250 ms p95 bar) or 1k (CI)
bench-embedding size="30k" out="":
    pnpm bench --embedding-only --embedding-size {{ size }} {{ if size == "30k" { "--max-p95-ms 250" } else { "" } }} {{ if out == "" { "" } else { "--out " + out } }}

# The quality gate (P2-17) against a real embedding model, run by its owner: set ATLAS_EVAL_EMBED_URL (e.g. http://localhost:8000/v1), ATLAS_EVAL_EMBED_MODEL, and ATLAS_EVAL_EMBED_KEY if the service needs one
eval-semantic:
    cargo test --manifest-path {{ manifest }} --locked --release --test semantic_eval -- --ignored --nocapture p2_17_the_vague_set_against_a_real_model

# Ask Atlas against real models, run by its owner (Slice 2.6): set the ATLAS_EVAL_EMBED_* variables as for eval-semantic plus ATLAS_EVAL_CHAT_MODEL; the chat model uses the same service and key unless ATLAS_EVAL_CHAT_URL/_KEY/_KIND say otherwise. Asks the vague set, then the off-library set; whole answers go to src-tauri/target/tmp/eval-ask-answers.md
eval-ask:
    cargo test --manifest-path {{ manifest }} --locked --release --test semantic_eval -- --ignored --nocapture ask_atlas_against_real_models

# The vector index alone (Phase 2): build, save, load, rebuild, recall and latency at `size` synthetic 1,024-dimension vectors (CI runs 100000)
bench-vectors size="300000" out="":
    pnpm bench --vectors-only --vectors-size {{ size }} {{ if out == "" { "" } else { "--out " + out } }}

# Ask a question set (fixtures/README.md) against a library, read-only, e.g. `just eval my-questions.json ~/AtlasData`; `ids` maps corpus ids (spikes/extraction-bench/corpus/manifest.json)
eval questions data_root ids="":
    cargo bench --manifest-path {{ manifest }} --locked --bench search -- --eval {{ questions }} --data-root {{ data_root }} {{ if ids == "" { "" } else { "--manifest " + ids } }}

# As eval, with meaning search through the library's embedding model (Atlas closed; the key from ATLAS_EVAL_EMBED_KEY or the keychain). The only network use of eval
eval-meaning questions data_root ids="":
    cargo bench --manifest-path {{ manifest }} --locked --bench search -- --eval {{ questions }} --data-root {{ data_root }} --semantic {{ if ids == "" { "" } else { "--manifest " + ids } }}

# WebDriver smoke test of the debug build (Windows and Linux; needs tauri-driver)
e2e *args:
    pnpm e2e {{ args }}

# ------------------------------------------- Slice 0.9 spike (spikes/)

# Download the pinned spike tools (PDFium, protoc, typst) into spikes/.tools
spike-tools:
    pnpm spike:tools

# Verify the spike corpus: checksums, sources, licenses
spike-corpus:
    pnpm spike:corpus

# Regenerate the generated part of the spike corpus (needs uv and spike-tools)
spike-corpus-make:
    cd spikes/extraction-bench/generator && uv run --python 3.12 make.py

# Extraction comparison (pdfium, pdf-extract, rbook); `docling` = path to a frozen sidecar to include it
spike-extract out="" docling="":
    pnpm spike:extract {{ if out == "" { "" } else { "--out " + out } }} {{ if docling == "" { "" } else { "--docling " + docling } }}

# Freeze the Docling sidecar (downloads PyTorch and Docling's models into the sidecar's build/ folder; about 2 GB)
spike-docling:
    cd spikes/extraction-bench/sidecar/docling && uv sync --locked --python 3.12 && uv run python build.py

# Vector-engine comparison, e.g. `just spike-vectors 1000000` (needs about 20 GB free at 1M)
spike-vectors n="100000" out="":
    pnpm spike:vectors --n {{ n }} {{ if out == "" { "" } else { "--out " + out } }}

# Clean build time and binary size of every spike candidate (slow: LanceDB)
spike-builds out="":
    pnpm spike:builds {{ if out == "" { "" } else { "--out " + out } }}

# Regenerate docs/spike-dependency-licenses.md
spike-licenses:
    pnpm spike:licenses

# Run CI on GitHub for the pushed current branch: `full` (every OS, smoke, benchmarks, bundles), `quick` (Linux checks and tests) or `evidence` (30k text extraction on every OS, every file read). Pushes to slice-* run quick by themselves
ci-remote scope="full":
    gh workflow run ci.yml --ref "$(git branch --show-current)" -f scope={{ scope }}

# Recent CI runs for the current branch
ci-status:
    gh run list --branch "$(git branch --show-current)" --limit 5

# --------------------------------------------------------- maintenance

# Regenerate docs/dependency-licenses.md
licenses:
    pnpm license-report

# Remove build outputs (Rust target dir and UI dist); keeps node_modules
clean:
    cargo clean --manifest-path {{ manifest }}
    node -e "require('fs').rmSync('dist', { recursive: true, force: true })"
