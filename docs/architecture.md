# Atlas architecture

Status: Phase 0, Slice 0.2. This document describes what is built, and marks
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
│    infrastructure/  volume inspector, atomic writes, clock     │
│                                                                │
│    catalog writer thread ── the only read-write SQLite conn    │
└────────────────────────────────────────────────────────────────┘
```

Tauri IPC is the private local API. Services are written so they do not
depend on Tauri types; a future, opt-in, authenticated, versioned HTTP API can
call the same service functions. See
[ADR 0001](adr/0001-local-ipc-no-server-single-data-root.md).

## IPC contract

Every Phase 0 command (spec section 9) is registered. Slice 0.2 added the
data-root and settings commands.

| Command group | Commands | Delivered by |
| --- | --- | --- |
| App | `get_app_status` | **0.1 (working)** |
| Data root | `get_data_root_status`, `get_default_data_root`, `pick_folder`, `inspect_data_root`, `choose_data_root`, `open_data_root` | **0.2 (working)** |
| Settings | `get_settings`, `update_settings` | **0.2 (working)** |
| Device | `get_device_snapshot` | 0.4 |
| Source roots | `list_source_roots`, `add_source_root`, `remove_source_root` | 0.3 |
| Inventory jobs | `start_inventory`, `pause_job`, `resume_job`, `cancel_job`, `get_inventory_status` | 0.5 |
| Library | `list_documents`, `open_original`, `reveal_original` | 0.6 |
| Providers | `list_provider_profiles`, `save_provider_profile`, `test_provider_connection` | 0.7 |

`get_app_status` returns only facts about the running process, including a
feature list that marks exactly which capabilities this build provides.
Commands whose slice has not shipped return a `not_implemented` error
naming itself and its planned slice.

Implemented commands are thin. They hand the work to a service on a blocking
thread (`AppState::with_workspace`), because data-root operations do
filesystem and SQLite I/O that must not run on the async executor.

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
snake_case strings, listed in `src-tauri/src/error.rs`. Every user-facing
code carries a `recovery` hint, and messages never include file paths.
The frontend adds two client-side codes:
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
| `CatalogRepository` | Source roots, file instances, later scan sessions | 0.3 / 0.5 |
| `InventorySource` | Streamed, read-only directory listing | 0.5 |
| `VolumeInspector` | Free space, filesystem type, network or cloud-synced | **0.2 (`OsVolumeInspector`)** |
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

- The only Tauri capability granted to the renderer is `core:default`. The
  dialog plugin is registered, but only Rust calls it (`pick_folder`), so the
  renderer has no dialog or filesystem permission. No shell, opener, or HTTP
  plugin is enabled.
- The renderer reaches native functionality only through Atlas commands, and
  the Rust side validates every ID and path.
- A strict Content Security Policy in `tauri.conf.json` allows only bundled
  scripts and IPC connections.
- Source files are never opened through string-built shell commands.

## Frontend

Svelte 5 with runes and Vite, without SvelteKit, because there is no SSR or
routing need. A small store selects the current view. At launch,
`src/lib/boot.ts` asks the core for the data-root status and routes:
not configured goes to Welcome and then the data-location screen, open goes
to Home, and missing, in use, incompatible or invalid goes to a recovery
screen. The theme is saved in the data root's `config/settings.json`;
browser storage is not used for Atlas preferences.

Accessibility baseline: landmarks and a skip link, `aria-current` on
navigation, focus moved to each view's heading, Cmd/Ctrl+1–3 view shortcuts,
visible focus rings, 44px major controls, `prefers-reduced-motion`, and
system light/dark via `prefers-color-scheme` with a `data-theme` override.

## Data root

Implemented in `src-tauri/src/services/workspace/`. Design rationale is in
[ADR 0002](adr/0002-data-root-creation-locking-and-compatibility.md).

```text
<root>/atlas.json             manifest: root ID, layout and schema versions (written last)
<root>/atlas.lock             exclusive OS lock while a process has the root open
<root>/config/settings.json   portable preferences (theme)
<root>/library/catalog.sqlite SQLite catalog, WAL mode (+ -wal, -shm)
<root>/tmp/                   staging for atomic writes; emptied on open
<root>/content/ indexes/ assets/ cache/ jobs/ models/ exports/ logs/ backups/   reserved, empty
```

- **Default location:** the OS per-user app-data folder. On macOS that is
  `~/Library/Application Support/com.altazlabs.atlas/Library`, on Windows
  `%LOCALAPPDATA%\Atlas\Library`, and on Linux `$XDG_DATA_HOME/atlas/library`.
- **Locator:** `locator.json` in the OS config folder records the root's path
  and ID. It is the one Atlas file outside the root. Debug builds honor
  `ATLAS_CONFIG_DIR` to redirect it for tests and smoke runs.
- **Inspecting** a location is read-only apart from a write probe. It reports
  free space, whether Atlas can write there, the volume kind, and what is
  already there: a new folder, an empty folder, an existing library, an
  interrupted setup, a folder with other files, or a file.
- **Creating** writes `.atlas-init`, takes the lock, creates the layout,
  catalog and settings, and writes `atlas.json` last and atomically. An
  interrupted creation is finished on the next attempt and keeps its ID.
- **Opening** never creates a root. It refuses a missing root, a damaged
  manifest, a newer layout or schema, a network or cloud-synced volume, and
  a root another process holds.
- **Catalog:** one writer thread owns the only read-write connection, with
  `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`, and a 5 second
  busy timeout. Migrations in `src-tauri/migrations/` run in order, each in
  its own transaction. The schema version is read before anything is
  written, so a newer catalog is refused untouched.
- **Data-root paths** must be absolute and valid Unicode. Source paths in
  later slices are stored as native bytes and are not restricted this way.

The webview also creates its own website-data and cache folders, on macOS
`~/Library/WebKit/<bundle>` and `~/Library/Caches/<bundle>`. These are
platform-required and contain no Atlas data.

## Reproducibility

- `rust-toolchain.toml` pins Rust 1.95.0. `Cargo.lock` is committed and CI
  passes `--locked`.
- `package.json` pins exact npm versions and the pnpm version.
  `pnpm-lock.yaml` is committed and CI uses `--frozen-lockfile`.
- GitHub Actions are pinned to full commit SHAs.
