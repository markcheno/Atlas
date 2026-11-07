# Platform validation

Record evidence here per slice. A platform is **verified** only when the app
was actually built and run on it. A passing compile, or CI configuration that
has never run, does not count.

## Slice 0.1

| Platform | Build | Launch | Shell walkthrough | Evidence |
| --- | --- | --- | --- | --- |
| macOS arm64 | ✅ dev and debug bundle | ✅ | ✅ | Below |
| Windows x64 | ❌ unverified | ❌ unverified | ❌ unverified | CI workflow written, never run (no remote yet) |
| Linux x64 | ❌ unverified | ❌ unverified | ❌ unverified | CI workflow written, never run (no remote yet) |

### macOS arm64 (2026-11-07)

Host: Apple Silicon (arm64), macOS (Darwin 27.0.0), Xcode installed, Rust
1.95.0, Node 24.16.0, pnpm 11.6.0.

- `pnpm check` passed. That covers rustfmt, clippy with `-D warnings`, 26 Rust
  unit tests, 4 Rust contract tests, ESLint, Prettier, svelte-check, and 9
  Vitest tests.
- `pnpm tauri dev` started the app. A 1100×720 window titled "Atlas" opened,
  with no errors in the dev log.
- Rendered content was verified through the macOS accessibility tree, because
  screen capture permission was not available to the automation session:
  - Welcome showed the headline, explanation, and a single Get Started button.
  - Get Started opened Home. The status bar showed
    "Atlas 0.1.0 · Phase 0 slice 0.1 · development build". That text comes
    from the real `get_app_status` IPC call.
  - Library showed the empty state and "planned for Phase 0 slice 0.6". That
    slice number comes from the real `not_implemented` error returned by
    `list_documents`.
  - Settings showed the theme radio group and the pending sections.
- `pnpm tauri build --debug --bundles app,dmg` produced `Atlas.app` and
  `Atlas_0.1.0_aarch64.dmg`. The bundled app launched standalone, with no dev
  server, and rendered Home with live IPC. Cmd+Q quit it, and no Atlas process
  remained.
- The only writes outside the repository were WebKit's own website-data and
  cache folders (`~/Library/WebKit/atlas` and `~/Library/Caches/atlas`). These
  are the documented OS-managed exception and contain no Atlas data.

Not verified on macOS: VoiceOver walkthrough, Intel (x86_64) Macs, minimum
supported macOS version, signed or notarized distribution, and opening the
`.dmg` on a second machine.

## Slice 0.2

| Platform | Build | Data root walkthrough | Evidence |
| --- | --- | --- | --- |
| macOS arm64 | ✅ | ✅ including a real exFAT drive unplug | Below |
| Windows x64 | ❌ unverified | ❌ unverified | The Windows volume code has never compiled. A cross-check from macOS failed because the bundled SQLite C build needs the Windows SDK. |
| Linux x64 | ❌ unverified | ❌ unverified | The Linux volume code has never compiled. |

### macOS arm64, Slice 0.2 (2026-11-07)

Same host as Slice 0.1. Every run used `ATLAS_CONFIG_DIR` pointed at a
scratch folder, and the real `~/Library/Application Support/com.altazlabs.atlas`
was confirmed absent afterwards.

- `just ci` passed. That covers 49 Rust unit tests, 4 contract tests, 19
  data-root integration tests, and 22 Vitest tests, plus rustfmt, clippy,
  ESLint, Prettier, svelte-check, and the bindings drift check.
- **First run.** Welcome led to the data-location screen. It showed the
  suggested path, "712 GB of 2.0 TB", "Atlas will create this folder", and
  "Can write here: Yes". Use this location created the root and opened Home.
  On disk the layout, manifest, locator, lock (containing the PID), WAL
  catalog, `schema_migrations` version 1, and the workspace row were all
  present. The workspace row's ID matched the manifest's root ID.
- **Theme persistence.** Choosing Dark wrote `"theme": "dark"` to
  `config/settings.json`. After a relaunch, Atlas went straight to Home and
  Settings showed Dark selected.
- **Second instance (P0-04).** A second Atlas process against the same root
  showed "Your library is open in another Atlas window" with the recovery
  hint, and `lsof` showed only the first process holding `atlas.lock`. After
  the first process was killed with SIGKILL, the lock had no holder. Try
  again in the second process opened the library, and that process then
  held the lock. `PRAGMA integrity_check` on the catalog returned `ok` after
  the kill.
- **Unplugged data drive (P0-14).** A 3 GB exFAT disk image was attached
  and a root was created on it. Atlas reported 3.2 GB free, WAL mode was
  active, and the lock was held on exFAT. After quitting and detaching the
  image, launching Atlas showed "Atlas can't find your library" with the last
  path and reconnect advice. Try again while still detached stayed on that
  screen, and `/Volumes/AtlasTest` was not recreated. After reattaching, Try
  again opened the library.

Not verified on macOS: the native folder picker. It was not driven by
automation; the non-empty-folder, nested-library, and subfolder-offer paths
are covered by integration and component tests instead. Also not verified:
SMB and NFS shares (no share was available; the fake-inspector tests cover
the refusal), iCloud Drive in practice, and the signed bundle.

## Manual checklist additions for Slice 0.2

On each platform, with `ATLAS_CONFIG_DIR` set to a scratch folder for a debug
build:

1. On first launch, Welcome leads to the data-location screen. Confirm the
   suggested path is the OS app-data folder, and that free space and "Can
   write here" are shown.
2. Choose another location and pick a folder that contains files. Confirm
   Atlas refuses it and offers "Create an “Atlas Library” folder here".
3. Use the location, quit, and relaunch. Confirm Atlas goes straight to Home,
   and that a theme chosen before quitting is restored.
4. Start a second copy against the same library. Confirm it shows the "in
   use" screen. Close the first copy, then press Try again in the second.
5. Put a library on a USB drive formatted as exFAT (NTFS on Windows, ext4 on
   Linux), quit, and unplug the drive. Relaunch and confirm the "can't find"
   screen appears and nothing is created at the old path. Replug the drive and
   press Try again.
6. If a network share is available, confirm it is refused as a data location.

## Manual checklist for Windows and Linux (Slice 0.1)

Run on a clean checkout and record the OS version, architecture, and outcome
of each step.

1. Install the prerequisites listed in the README for the platform.
2. `pnpm install --frozen-lockfile`
3. `pnpm check`, which must pass with no failures.
4. `pnpm tauri dev`, then confirm a window titled "Atlas" opens on Welcome.
5. Press Tab, and confirm focus is visible. Activate Get Started with Enter or
   Space.
6. Confirm the status bar shows "Atlas 0.1.0 · Phase 0 slice 0.1 · development build".
   This proves IPC works.
7. Press Ctrl+2 and confirm Library shows "planned for Phase 0 slice 0.6".
8. Press Ctrl+3, choose Dark, then Light, then Match system. Confirm the
   theme follows, and that Match system tracks the OS setting.
9. Close the window. Confirm no `atlas` process remains (Task Manager, or
   `pgrep atlas`).
10. Windows: `pnpm tauri build --debug --bundles nsis`, then install the setup
    `.exe`, launch Atlas from the Start menu, and repeat steps 4–9. Note any
    SmartScreen prompt.
11. Linux: `pnpm tauri build --debug --bundles appimage,deb`, then run the
    AppImage and install the `.deb`, repeating steps 4–9 for each. Record the
    distribution, desktop environment, and WebKitGTK version.

## Known platform notes

- **Linux:** WebKitGTK 4.1 is required at runtime. CI builds on Ubuntu 24.04,
  so its AppImage needs glibc 2.39 or newer. Older distributions are not
  claimed as supported.
- **Windows:** The NSIS installer depends on the system WebView2 runtime,
  which ships with current Windows 10 and 11.
- **All platforms:** Bundles are unsigned and not notarized in Phase 0, so OS
  security prompts are expected.
- **Icons:** The bundles still use Tauri's default placeholder icons.
