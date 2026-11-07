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
