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

# Install the pinned Rust toolchain and locked npm dependencies
setup:
    rustup toolchain install
    pnpm install --frozen-lockfile

# ------------------------------------------------------------------ run

# Launch the desktop app with hot reload
dev:
    pnpm tauri dev

# Serve only the UI in a browser (native commands report unavailable)
ui:
    pnpm dev

# Launch with no cataloging rate limit, for benchmarks (debug builds only)
dev-unthrottled $ATLAS_INVENTORY_UNTHROTTLED="1":
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

# Generate a synthetic test library outside the repo, e.g. `just fixtures ~/atlas-fx 10000 20000` (30k with a 20k flat folder)
fixtures dir files="1000" flat="0":
    node scripts/generate-fixtures.mjs {{ dir }} --files {{ files }} --flat {{ flat }}

# ------------------------------------------------- benchmarks and smoke

# Benchmark suite (1k and 30k fixtures, release build); results in the temp dir unless `out` is given
bench out="":
    pnpm bench {{ if out == "" { "" } else { "--out " + out } }}

# Benchmark suite plus the 30k throttle sweep (about 10 more minutes)
bench-full out="":
    pnpm bench --sweep {{ if out == "" { "" } else { "--out " + out } }}

# WebDriver smoke test of the debug build (Windows and Linux; needs tauri-driver)
e2e *args:
    pnpm e2e {{ args }}

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
