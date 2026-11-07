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

## Slice 0.3

| Platform | Build | Library folders walkthrough | Evidence |
| --- | --- | --- | --- |
| macOS arm64 | ✅ dev and `.app` | ✅ including the native picker and an exFAT unplug | Below |
| Windows x64 | ❌ unverified | ❌ unverified | Path encoding and `same-file` identity untested on Windows |
| Linux x64 | ❌ unverified | ❌ unverified | The non-UTF-8 folder test exists but runs only on Linux |

### macOS arm64, Slice 0.3 (2026-11-07)

Every run used `ATLAS_CONFIG_DIR` and a scratch source tree; no real
documents were read.

- `just ci` passed. That covers 58 Rust unit tests, 4 contract tests, 19
  data-root tests, 15 source-root tests (the Linux-only non-UTF-8 test does
  not run on macOS), and 29 Vitest tests. The case-insensitive alias test
  ran, because APFS here is case-insensitive.
- **Onboarding.** After the data location was created, the Library folders
  step appeared with the read-only promise and Continue disabled.
- **Native picker.** It was driven with keystrokes (⌘⇧G, path, ⌘A, Return).
  Selecting `Books` and `Papers` in one pick added both, each shown as
  Available. Picking `Books/Astronomy` was refused with "That folder
  overlaps a folder already in your library. (“Books”)" and the recovery
  hint. Cancelling the picker changed nothing.
- **Persistence.** After quitting and relaunching, Home listed both folders.
- **Removal.** In Settings, Remove showed the inline "Remove from Atlas? Your
  files stay where they are." confirmation, and confirming deleted only the
  catalog row. A `stat` snapshot of the source tree (names, sizes, mtimes)
  was identical before and after.
- **Unplugged source.** A folder on an exFAT disk image was added through
  the picker. The image ejected cleanly while Atlas was running, because
  Atlas holds no handles on source drives. Home then showed the folder as
  "Not connected" and kept it listed. After reattaching, it showed
  Available again.
- **Privacy strings.** `plutil -p Atlas.app/Contents/Info.plist` lists all
  five usage descriptions.

Not verified on macOS:

- **Data root through the picker.** Picking the data root was not driven
  manually, because keystroke delivery to the panel was unreliable on that
  attempt. The refusal is covered by integration tests.
- **Privacy prompt.** The live prompt for a protected folder was not
  observed, because that would mean reading the user's real Documents
  folder.
- **Network share.** Adding a folder on a network share was not tried.

## Slice 0.4

| Platform | Build | Device check walkthrough | Evidence |
| --- | --- | --- | --- |
| macOS arm64 (laptop, on battery) | ✅ | ✅ real readings plus five simulated states | Below |
| Windows x64 | ❌ unverified | ❌ unverified | Battery probe, desktop detection, and unknown pressure and thermal untested |
| Linux x64 | ❌ unverified | ❌ unverified | The PSI memory-pressure parser is unit-tested only on Linux |

### macOS arm64, Slice 0.4 (2026-11-07)

All runs used a scratch `ATLAS_CONFIG_DIR`.

- `just ci` passed. That covers 82 Rust unit tests (including the P0-10,
  P0-11 and P0-12 policy tests), 3 device integration tests, 4 contract
  tests, 15 source tests, 19 data-root tests, and 40 Vitest tests.
- **Real readings.** These were compared with OS tools:

  | Reading | Atlas | OS tool |
  | --- | --- | --- |
  | Memory | 64 GB | `sysctl hw.memsize` 68,719,476,736 |
  | Processor | 18 cores | `sysctl hw.logicalcpu` 18 |
  | Data drive | 710 GB free of 2.0 TB | `df -k` 693,025,504 KiB available of 1,948,404,040 |
  | Power | On battery · 72% | `pmset -g batt` 72%, discharging |

  The recommendation said the computer "can likely run AI models locally".
  The on-battery note and the provisional caveat were shown.
- **Live policy.** Settings › Performance & Battery read: "When Atlas
  catalogs your folders (coming in slice 0.5), it will run slowly because
  this computer is on battery."
- **Simulated states.** Each was launched with `just dev-simulate`:

  | Simulation | Power shown | Policy sentence |
  | --- | --- | --- |
  | `battery:18` | On battery · 18% | pause because the battery is at 18%. It resumes at 25% or when you plug in. |
  | `battery:26` | On battery · 26% | run slowly because this computer is on battery. |
  | `unknown` | Unknown | run slowly because Atlas can't tell whether this computer is plugged in. |
  | `desktop` | No battery (desktop) | run normally. |
  | `ac 1000000` | Plugged in · 100%, "1.0 MB free of 2.0 TB" | pause because the drive holding Atlas's data is nearly full. It resumes when at least 10.7 GB is free. |

Not verified:

- **Real unplug and replug.** No physical power change was made. The
  hysteresis sequence (19, 20, 21, 24, then 25%) is unit-tested, because
  readings within one simulated launch are constant.
- **Thermal and memory pressure transitions.** Only the nominal and normal
  readings were observed.
- **macOS "unknown" while holding charge on AC.** Not observed; documented
  in ADR 0004.

## Slice 0.5

| Platform | Build | Cataloging walkthrough | Evidence |
| --- | --- | --- | --- |
| macOS arm64 | ✅ | ✅ including a real `kill -9` mid-scan | Below |
| Windows x64 | ❌ unverified | ❌ unverified | Cloud-placeholder attributes, junction skipping, and `TerminateProcess` in the crash test are untested |
| Linux x64 | ❌ unverified | ❌ unverified | Untested |

### macOS arm64, Slice 0.5 (2026-11-07)

All runs used scratch `ATLAS_CONFIG_DIR` folders and generated fixtures
(`just fixtures`); no real documents were read.

- **`just ci` passed.** That covers 95 Rust unit tests and 19 inventory
  integration tests, which include P0-05/06/07/09/10/11/12/13/20/21/22,
  rescans, a 20,000-entry folder, cancel, removal mid-scan, iCloud
  placeholders, and never-scanned folders. It also covers the real-kill
  P0-08 test, 3 device, 4 contract, 15 source, and 19 data-root tests, and
  52 Vitest tests. The inventory suite was run 5 times and the crash test
  3 times, with no flakes.
- **2,000-file fixture.** Run at simulated AC (`just dev-simulate ac`) and
  added through the native picker during onboarding. The status bar counted
  up, then showed "Up to date · 2,006 files", matching the generator's
  count exactly. A `stat` snapshot of the fixture was identical before and
  after.
- **Pause across restart (P0-09).** On the 30,000-file fixture, Pause
  showed "Paused by you · 1,992 files so far", and the row count stayed at
  3,998 over 4 s. After quitting and relaunching it was still paused with
  the same count. Resume continued from there.
- **Battery policy.** Relaunching with `just dev-simulate battery:18`
  mid-scan showed "Paused on battery · Resumes at 25% or when plugged in".
  The session read `paused_policy | battery_low`, and no rows were written
  over 5 s.
- **Real crash (P0-08).** A throttled scan of 30,006 files was killed with
  `kill -9` at 1,796 rows. The session was left `running` and
  `PRAGMA integrity_check` returned `ok`. After relaunching, it completed
  with 30,006 rows and 30,006 distinct paths. None of the 1,796 pre-kill
  rows changed ID, and no frontier rows remained.
- **Deletion.** After deleting one fixture file and pressing Rescan, exactly
  that file became `missing`. The status bar showed "1 file missing" and
  the folder row read "30,005 files · Updated just now · 1 missing". The
  fixture's unreadable `locked/` folder was retried 3 times and did not
  block this.

**First measurements** (warm filesystem cache, M-series MacBook on battery,
debug build):

| Measure | Result |
| --- | --- |
| Unthrottled full scan, 30,006 files in 352 folders | 1.0–1.1 s of running time |
| Throttled rate | 100 metadata ops/s on AC (about 5 min for 30k), 20/s on battery (by policy) |
| Process RSS, idle and during a 30k rescan | 142 MB and 142 MB (no measurable growth) |
| Catalog size for 30,006 files | 24 MB database, 5 MB WAL |

Not verified:

- **Real iCloud dataless files and folders.** Atlas didn't read your
  iCloud Drive, and test files can't be made dataless without a File
  Provider. The legacy `.icloud` placeholder path is covered by tests.
- **Cold-cache and external-drive throughput.** Deferred to Slice 0.8.
- **Wake from sleep mid-scan.**

## Manual checklist additions for Slice 0.5

1. Generate a fixture with `just fixtures <dir> 2000`, add it, and confirm
   the final count equals `catalogableFiles`.
2. Pause during a scan, quit, relaunch, and confirm it is still paused.
   Resume.
3. Kill the process during a scan (Task Manager, or `kill -9`), relaunch,
   and confirm it completes with no duplicate rows.
4. Unplug a drive holding a library folder during a scan. Confirm the
   status says the folder isn't available and no files show as missing.
   Plug it back in and confirm it resumes.
5. Windows: add a OneDrive folder with Files On-Demand. Confirm online-only
   files are cataloged without being downloaded (the cloud icon stays).
6. macOS: add an iCloud Drive folder with "Optimize Mac Storage". Confirm
   offloaded files are cataloged and stay offloaded.

## Manual checklist additions for Slice 0.4

1. Reach the Device check step during onboarding. Compare memory, cores,
   free space, and power with the OS tools: Activity Monitor or Task Manager,
   and `df` or Explorer.
2. On a laptop, unplug the power, open Settings › Performance & Battery, and
   wait up to 30 seconds. Confirm it says it will "run slowly because this
   computer is on battery". Plug in again and confirm it says "run normally".
3. On a desktop with no battery, confirm Power reads "No battery (desktop)"
   and the policy says "run normally".
4. Debug build: run `just dev-simulate battery:18`, then `unknown`, then
   `ac 1000000`. Confirm the pause, slow, and low-space sentences shown above.

## Manual checklist additions for Slice 0.3

1. Finish onboarding to the Library folders step. Confirm the read-only text
   appears, and that Continue stays disabled until a folder is added.
2. Add two folders in one pick. Then pick a folder inside one of them, and
   confirm Atlas refuses it and names the outer folder.
3. Pick the Atlas data folder itself, and confirm it is refused.
4. Add a folder on a USB drive, then unplug the drive. Confirm it shows "Not
   connected". Plug it back in and confirm it shows Available.
5. Remove a folder in Settings and confirm. Check that the folder and its
   files are unchanged on disk.
6. Windows: add `C:\Users\<you>\Documents\Books`, then try the same folder
   with different capitalization, and confirm it is refused as already
   added.
7. Linux: add a folder whose name is not valid UTF-8, for example created
   with `mkdir $'caf\xe9'`. Confirm it is listed with a replacement character
   and shows Available.

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
