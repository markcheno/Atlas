# Atlas architecture

Status: Phase 0, Slice 0.1. This document describes what is built, and marks
what is only planned. Decisions with lasting consequences are recorded as
ADRs in [adr/](adr/).

## Process model

Atlas is one desktop process. The Tauri 2 runtime hosts the native window, an
OS webview that renders the Svelte UI, and the Rust core. There is no HTTP
server, background daemon, database server, or Python runtime.

```text
┌──────────────────────── Atlas process ────────────────────────┐
│  OS webview (WKWebView / WebView2 / WebKitGTK)                 │
│    Svelte UI ── src/lib/api/client.ts (typed facade)           │
│                         │  Tauri IPC (invoke)                  │
│  Rust core              ▼                                      │
│    commands/   thin adapters: validate input, map DTOs         │
│        │                                                       │
│    services/   workspace · catalog · inventory · scheduler     │
│                system · providers · source_open                │
│        │                                                       │
│    domain/     DTOs (ts-rs exported) and service traits        │
│    infrastructure/  OS, SQLite and HTTP adapters (from 0.2)    │
└────────────────────────────────────────────────────────────────┘
```

Tauri IPC is the private local API. Services are written so they do not
depend on Tauri types; a future, opt-in, authenticated, versioned HTTP API can
call the same service functions. See
[ADR 0001](adr/0001-local-ipc-no-server-single-data-root.md).

## IPC contract

All eighteen Phase 0 commands (spec section 9) are registered from Slice 0.1.

| Command group | Commands | Delivered by |
| --- | --- | --- |
| App | `get_app_status` | **0.1 (working)** |
| Device | `get_device_snapshot` | 0.4 |
| Data root | `choose_data_root`, `open_data_root` | 0.2 |
| Source roots | `list_source_roots`, `add_source_root`, `remove_source_root` | 0.3 |
| Inventory jobs | `start_inventory`, `pause_job`, `resume_job`, `cancel_job`, `get_inventory_status` | 0.5 |
| Library | `list_documents`, `open_original`, `reveal_original` | 0.6 |
| Providers | `list_provider_profiles`, `save_provider_profile`, `test_provider_connection` | 0.7 |

`get_app_status` returns only facts about the running process: version, OS,
architecture, build profile, slice, and a feature list in which every entry
is `implemented: false`. Every other command returns a `not_implemented`
error naming itself and its planned slice.

**Single source of truth for types.** DTOs are defined once in
`src-tauri/src/domain/dto.rs`. `ts-rs` writes matching TypeScript to
`src/lib/api/bindings/` during `cargo test`, and CI fails if the committed
bindings drift. Conventions: camelCase fields, snake_case enum values,
RFC 3339 timestamps, opaque UUID string IDs, byte counts as JSON numbers, and
`null` for absent values.

**Contract tests.** `src-tauri/tests/ipc_contract.rs` checks that every
command is registered, listed in `COMMAND_NAMES`, invoked by the frontend
facade under the same name, and that no stub returns data.

**Errors.** One `AtlasError` type serializes to
`{ code, message, recovery?, command?, plannedSlice? }`. Codes are stable
snake_case strings. The frontend adds two client-side codes:
`ipc_unavailable` when running outside the desktop app, and `unexpected`
for any malformed error, which never echoes the raw error text.

**Paths over IPC.** The renderer only ever sees display strings for paths.
Native path bytes stay in Rust (spec section 6).

## Service interfaces

Traits exist only where tests need a fake, or to reserve a name (spec
section 9, abstraction budget). They are defined in
`src-tauri/src/domain/traits.rs` and have no implementations yet.

| Trait | Purpose | First adapter |
| --- | --- | --- |
| `CatalogRepository` | Source roots, file instances, later scan sessions | 0.2 / 0.3 / 0.5 |
| `InventorySource` | Streamed, read-only directory listing | 0.5 |
| `ResourceMonitor` | Device snapshot for the data-root volume | 0.4 |
| `ModelProvider` | Endpoint probe and model listing | 0.7 |
| `SourceOpener` | OS-native open and reveal | 0.6 |
| `DocumentProcessor` | Reserved name for Phase 1 extraction | Phase 1 |

The scheduler is deliberately concrete. With one job kind in Phase 0 there is
no `JobProcessor` or `SchedulingPolicy` trait; those are introduced with the
second job kind.

`ModelProvider::test_connection` returns `impl Future + Send` instead of using
`async fn`, so callers can rely on `Send`. Traits are not yet required to be
object-safe; whether adapters are held as generics or `dyn` is decided when
the first adapter lands.

## Security boundary

- The only Tauri capability granted is `core:default`. No filesystem, shell,
  dialog, opener, or HTTP plugin is enabled. Each later slice adds the
  narrowest plugin permission it needs, for example the dialog plugin in 0.2.
- The renderer reaches native functionality only through Atlas commands, and
  the Rust side validates every ID and path.
- A strict Content Security Policy in `tauri.conf.json` allows only bundled
  scripts and IPC connections.
- Source files are never opened through string-built shell commands.

## Frontend

Svelte 5 with runes and Vite, without SvelteKit, because there is no SSR or
routing need. A small store selects one of four views. In Slice 0.1 Welcome
shows on every launch, because first-run state can only be remembered once
the data root exists (0.2). The theme is held in memory for the same reason;
browser storage is not used for Atlas preferences.

Accessibility baseline: landmarks and a skip link, `aria-current` on
navigation, focus moved to each view's heading, Cmd/Ctrl+1–3 view shortcuts,
visible focus rings, 44px major controls, `prefers-reduced-motion`, and
system light/dark via `prefers-color-scheme` with a `data-theme` override.

## Data root

Not implemented in Slice 0.1; nothing is written to disk by Atlas itself.
Slice 0.2 introduces the layout in spec section 6. One OS-managed exception
is already visible: the webview creates its own website-data and cache
folders (on macOS, `~/Library/WebKit/<bundle>` and
`~/Library/Caches/<bundle>`). These are platform-required and contain no
Atlas data.

## Reproducibility

- `rust-toolchain.toml` pins Rust 1.95.0. `Cargo.lock` is committed and CI
  passes `--locked`.
- `package.json` pins exact npm versions and the pnpm version.
  `pnpm-lock.yaml` is committed and CI uses `--frozen-lockfile`.
- GitHub Actions are pinned to full commit SHAs.
