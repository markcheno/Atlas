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

## Slice 0.6

| Platform | Build | Library walkthrough | Evidence |
| --- | --- | --- | --- |
| macOS arm64 | ✅ | ✅ search, open in Preview, show in Finder, missing file, iCloud placeholder | Below |
| Windows x64 | ❌ unverified | ❌ unverified | Explorer reveal (`SHOpenFolderAndSelectItems`) and `ShellExecuteW` untested |
| Linux x64 | ❌ unverified | ❌ unverified | `xdg-open` and FileManager1 reveal untested |

### macOS arm64, Slice 0.6 (2026-11-07)

All runs used a scratch `ATLAS_CONFIG_DIR` and a fresh 2,000-file fixture
whose PDFs are now valid one-page documents.

- **`just ci` passed.** That covers 99 Rust unit tests, 7 library tests, 8
  open/reveal tests, the earlier suites, and 61 Vitest tests. The library
  tests include P0-15, folding, filters, sorts, paging, backfill, an
  unplugged folder, and query latency. The open/reveal tests include P0-16
  shell-hostile names, a missing file, an unplugged folder, a symlink swap,
  an escape through a parent link, and a legacy placeholder.
- **Query latency.** On a 30,000-file catalog, 50 mixed queries (text,
  filters, sort, page 50, no-match) ran at p50 31 ms, p95 33 ms, max 35 ms
  in a debug build. The target is p95 under 250 ms.
- **During the scan.** The Library showed "Showing 1–50 of 1,980 matches so
  far", and after completion "Showing 1–50 of 2,006 matches", equal to the
  generator's count.
- **Search.** Typing `cafe` listed the 15 "Café crème …" files, matching the
  catalog's folded-key count.
- **Open.** Preview opened "Café crème book 000136.pdf – 1 page". Only that
  window was closed afterwards.
- **Show in Finder.** Finder's front window was `level-0-3`, with that file
  selected. Only that window was closed afterwards.
- **Missing file.** After deleting a file, Open showed "That file is no
  longer where Atlas found it." with the advice to rescan its folder.
- **Legacy iCloud placeholder.** After adding `.Cosmos.pdf.icloud` (size
  48,213,004) and rescanning, the row read "Cosmos.pdf · 48.2 MB · Only in
  the cloud". Open explained that the file "is stored only in iCloud and
  must be downloaded" and suggested showing it in Finder. Show in Finder
  selected `.Cosmos.pdf.icloud`.

Not verified:

- **Download on open.** The "Download and open" path for a real dataless
  iCloud file was not observed; Vitest and the Rust checks cover the
  protocol.
- **Unplugged-folder rows in the app.** Not repeated by hand; the
  integration test covers the `unreachable` status and the open refusal.
- **Accessibility tree duplicates.** WebKit exposes table cells under both
  rows and columns, so the tree shows content more than once. This is an
  inspection artifact; the DOM has a single copy, which the tests confirm.

## Slice 0.7

| Platform | Build | AI Models walkthrough | Evidence |
| --- | --- | --- | --- |
| macOS arm64 | ✅ | ✅ against real Ollama 0.40.15 and oMLX | Below |
| Windows x64 | ❌ unverified | ❌ unverified | Credential Manager store and rustls/aws-lc build untested |
| Linux x64 | ❌ unverified | ❌ unverified | Secret Service store untested; without a running Secret Service, keys fall back to session-only |

### macOS arm64, Slice 0.7 (2026-11-07)

All runs used a scratch `ATLAS_CONFIG_DIR` and the debug keychain service
`com.altazlabs.atlas.debug`. No model was downloaded and no real key was
used.

- **`just ci` passed.** That covers 106 Rust unit tests, 8 provider
  integration tests against a scripted local HTTP server, all earlier
  suites, and 68 Vitest tests. The provider tests cover P0-17, P0-18,
  P0-19, empty lists, every failure class, timeouts, redirects not
  followed, session-only keys, and detection. The provider tests were run
  3 times with no flakes.
- **Look for AI services on this computer** found `Ollama (http://localhost:11434)` and
  `oMLX (http://localhost:8000/v1) · needs a key`. oMLX was identified by its `/health` response.
- **Ollama, real.** Test reported "Ollama 0.40.15 is running but has no
  models installed. Install one with Ollama, then test again." The detail
  was "The service answered with an empty model list."
- **oMLX, real, no key.** Test reported "oMLX needs an API key. Enter the
  key from oMLX's settings." (HTTP 401).
- **oMLX, real, dummy key.** With the key `atlas-dummy-key-not-real-7f3a`,
  Test reported "The key was rejected" (HTTP 401). After saving, the card
  read "Key saved in Keychain".
  - `security find-generic-password` (attributes only) showed a `genp` item
    with service `com.altazlabs.atlas.debug` and account
    `provider:<profile id>`. The release service had no item.
  - The key string was absent from `catalog.sqlite`, its `-wal` and `-shm`
    files, every file under the library folder, and the dev log.
  - Testing the saved profile read the key from the Keychain; it was
    rejected again, which shows it was sent.
  - Remove deleted both the row and the Keychain item.
- **Plain HTTP to another computer.** Entering `http://192.168.1.20:11434`
  showed the warning and the "I trust this network" checkbox. Test
  reported "Unencrypted HTTP to another computer needs your confirmation
  first."

Not verified:

- **Authenticated success against oMLX.** The user enters the real key
  in Settings › AI Models. A mock covers the success path.
- **A real model listing from Ollama.** No model was downloaded, by the
  user's choice. A mock covers it.
- **Hosted HTTPS services** (OpenAI and others), and the keychain
  access prompt that an installed (signed) build may show.

## Slice 0.8

| Platform | CI checks and tests | Bundles | Smoke | Benchmarks |
| --- | --- | --- | --- | --- |
| macOS arm64 | ✅ CI | ✅ `.app`, `.dmg` (debug and release) | ✅ manual, release DMG, on a real Mac | ✅ local and CI |
| Windows x64 | ✅ CI | ✅ NSIS `-setup.exe` (debug and release) | ✅ automated, WebView2 | ✅ CI |
| Linux x64 | ✅ CI | ✅ `.AppImage`, `.deb` (debug and release) | ✅ automated, Xvfb | ✅ CI |

CI run: [42488722770](https://github.com/markcheno/Atlas/actions/runs/42488722770)
(2026-11-08, branch `slice-08`), all nine jobs green; benchmarks are from
the previous run, [42487239937](https://github.com/markcheno/Atlas/actions/runs/42487239937); the main-branch run
after merging is linked in the README. Benchmark numbers are in
[benchmarks.md](benchmarks.md) and bundle sizes in
[packaging.md](packaging.md).

### Automated smoke (Windows and Linux CI)

`e2e/smoke.mjs` drives the debug build through `tauri-driver` (WebKitWebDriver
on Linux, msedgedriver on Windows) with a scratch configuration, a scratch
data folder, and a fresh 1,000-file fixture. Debug-only hooks
(`src-tauri/src/infrastructure/e2e.rs`) answer the two native folder pickers
with fixed paths and record open/reveal requests in a file instead of
calling the OS. Release builds compile the hooks out.

Steps: launch shows the welcome screen; choose the data folder; add the
fixture and see Library rows; pause and resume; cataloging finishes with
the expected count; search "CAFE creme" finds "Café crème …"; Open sends the
right path (inside the folder, existing); Settings shows AI Models; quit and
relaunch reopens the same library without onboarding. Screenshots and
`e2e-<os>.json` are uploaded as the `e2e-<OS>` artifact.

| | Linux x64 (Ubuntu 24.04, Xvfb) | Windows x64 (Server 2025, build 26100) |
| --- | ---: | ---: |
| Result | ✅ all 9 steps | ✅ all 9 steps |
| Launch to welcome screen | 0.96 s | 2.6 s |
| Folder selection to first rows (clicks included) | 0.53 s | 0.37 s |
| Cataloging 1,006 files after resume | 2.9 s | 3.4 s |
| Relaunch to the reopened library | 1.0 s | 1.4 s |
| Whole run | 25 s | 27 s |

These are debug builds on shared runners, so treat the timings as rough.

Three problems were found and fixed on the way:

- **Windows paths:** the first passing run's screenshots showed every path
  in Windows' verbatim form (`\\?\C:\Users\…`), because folders were
  canonicalized with `std::fs::canonicalize`. Atlas now uses
  `dunce::canonicalize`, which gives the ordinary `C:\…` form whenever
  that is unambiguous. The smoke test fails if a verbatim path is shown or
  sent to the OS.
- **Linux:** without a D-Bus session, every launch waited about 25 s for the
  accessibility bus (32 s to the welcome screen). The smoke test now runs
  under `dbus-run-session`, as a real desktop would; 0.95 s.
- **Windows:** msedgedriver couldn't attach ("DevToolsActivePort file
  doesn't exist"), because wry always sets WebView2's browser arguments and
  so overrides the remote-debugging arguments the driver passes through
  `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`. The main window is now created in
  code, and debug builds add the driver's arguments to wry's defaults
  (`infrastructure/e2e.rs`). Release builds get wry's defaults only, as
  before.

### macOS arm64, Slice 0.8 (2026-11-08)

MacBook Pro (Mac17,9), Apple M5 Pro, 18 cores, 64 GB, macOS 27.0.1. The
**release** bundle (`just bundle-release`) was mounted from its DMG and launched directly with
a scratch `HOME`, so the real configuration and library were never read or
written (checked before and after). Driven through the accessibility tree;
timings are from polling and are good to about ±300 ms.

- **Launch to the welcome screen:** 1.45 s. The suggested data folder was
  under the scratch home.
- **Data folder** picked through the real macOS folder dialog; the library
  was created there.
- **Library folder:** the 30,006-file fixture added through the real dialog.
  On battery power, the status bar read "Slowed on battery".
- **Pause:** "Paused by you · 24,673 files so far", unchanged 3 s later.
  Quit from the app menu and relaunched: still paused (relaunch to the
  paused status: 2.0 s). Resume then finished at "Up to date · 30,006 files".
- **Search** "CAFE creme" found the "Café crème" files.
- **Open** showed "Café crème book 000136.pdf" in Preview; **Show in
  Finder** selected the file in its folder. Only those windows were closed
  afterwards.
- **Quit Atlas** left no process running.
- **CI-built DMG:** downloaded from the run's artifact (so quarantined),
  `codesign --verify --deep --strict` passes (ad-hoc, hardened runtime,
  sealed resources). `spctl --assess` rejects it, as expected for an
  unnotarized app. The Gatekeeper dialog itself was not opened: that needs a
  Finder launch, which would use the real profile.

### Not verified in Slice 0.8

- **Windows and Linux native folder dialogs** and **opening or revealing
  files in the OS**: the smoke test bypasses both (by design; see above).
- **Windows Credential Manager and Linux Secret Service** storing a real key.
- **Real removable-drive unplugging and real battery hardware** on Windows
  and Linux (simulated in tests).
- **Windows long paths** (over 260 characters): cataloging and reading are
  tested in CI since Slice 1.7; opening one through the Windows shell, and
  network shares, are not.
- **Installing** the Windows and Linux bundles on a real desktop, including
  SmartScreen; the Gatekeeper dialog on macOS.
- **Other Linux distributions**, Wayland, and ARM.

The manual checklist below covers these.

## Slice 0.9

Slice 0.9 measures libraries rather than adding to the app; the app only
reports "slice 0.9". Its evidence is the spike harness running on all three
OSes. Results are in [the spike results](spike-0.9.md) and the decision is
[ADR 0008](adr/0008-extraction-and-vector-engines.md).

| Platform | Extraction (pdfium, pdf-extract, rbook) | Docling sidecar | Vector engines (100k) | Build cost |
| --- | --- | --- | --- | --- |
| macOS arm64 | ✅ CI and M5 Pro | ✅ CI and M5 Pro (built, ran the corpus) | ✅ CI; M5 Pro also at 1M | ✅ CI and M5 Pro |
| Windows x64 | ✅ CI | ✅ CI | ✅ CI | ✅ CI |
| Linux x64 | ✅ CI | ✅ CI | ✅ CI | ✅ CI |

CI: Spikes workflow run
[42505069661](https://github.com/markcheno/Atlas/actions/runs/42505069661)
(2026-11-08, branch `slice-09`), 12 of 12 jobs green. On every OS:
- the extraction scores were identical;
- no candidate crashed or hung on any malformed file;
- every vector engine built and passed its delete and crash tests.

App checks: `just ci` passed on the M5 Pro after the slice number moved to
0.9; app CI runs on the merge to `main` as usual.

**Not verified in Slice 0.9:**
- extraction of the user's own documents (by design, only the committed
  corpus was used);
- real embeddings;
- OCR;
- ARM Windows and Linux;
- Docling's peak memory on Windows (the measurement read 0).

## Phase 1

Phase 1 slices are recorded below, in the same form as Phase 0's above
([Phase 1 spec](phase1-spec.md) §15).

## Slice 1.1

Slice 1.1 adds no screens. The catalog's `scan_sessions` table becomes
`jobs` with a `kind` (migration 0007), scans run on the general job runner,
and the status bar reads "Phase 1 slice 1.1". Automated evidence is P1-01
and P1-02 (`src-tauri/tests/jobs.rs`), which passed 3 runs of 3 locally,
plus the 6→7 migration unit test with foreign-key checks. Removing the scan
filter from the cataloging overview, or from the Library's "scanning"
flag, makes P1-02 fail.

CI on branch `slice-11` (2026-11-08): the push ran the new quick tier,
[42525777085](https://github.com/markcheno/Atlas/actions/runs/42525777085)
(Linux checks and tests, green in 5 min 15 s from a cold cache). The full
tier, dispatched with `just ci-remote` as
[42526247859](https://github.com/markcheno/Atlas/actions/runs/42526247859),
ran 10 of 10 jobs green (the plan job and 9 on the three OSes) in 10 min 14 s: checks, tests and debug bundles on
three OSes, smoke tests on Windows and Linux, benchmarks, and release
bundles. The full run built from cold because the check jobs moved to a
shared cache key. A docs-only push then ran the docs tier,
[42527143995](https://github.com/markcheno/Atlas/actions/runs/42527143995):
plan and lint only, green in 35 s. After the merge, the full tier passed
on `main`
([42527727223](https://github.com/markcheno/Atlas/actions/runs/42527727223))
and on the tag `phase1/slice-01`
([42527727023](https://github.com/markcheno/Atlas/actions/runs/42527727023)),
10 of 10 jobs each.

### macOS arm64, Slice 1.1 (2026-11-08)

Upgrade of a real Phase 0 library, using scratch copies only:

1. With `main`'s Phase 0 debug build and a scratch `ATLAS_CONFIG_DIR`,
   created a library, added the 30k benchmark fixture, and paused the scan
   by hand at 5,900 files (2 folders done, 9 pending). Quit, then copied
   the library folder and pointed a second scratch locator at the copy.
2. Opened the copy with the Slice 1.1 debug build. The status read
   "Cataloging paused · Paused by you · 5,900 files so far", and the status
   bar "Atlas 0.1.0 · Phase 1 slice 1.1 · development build". The catalog
   held migrations 1–7, `atlas.json` said 7, and the job was `kind =
   'scan'`, still `paused_user`, with its frontier unchanged.
3. Resume finished in about 55 s at the policy rate: 30,006 files, all
   available, none missing, no duplicate rows, and the frontier was
   cleared. The 3 failures are the fixture's unreadable `locked/` folder
   (3 attempts), as in Phase 0.
4. The Library showed "Showing 1–50 of 30,006 matches".
5. After quitting, opened the migrated copy with `main`'s Phase 0 build. It
   showed "This library needs a newer version of Atlas … (format 7; this
   version supports up to 6)". The catalog and `atlas.json` stayed
   byte-identical (SHA-256). Only `atlas.lock` was rewritten; it records
   the process holding the library and is taken before the schema check
   (ADR 0002).

Benchmarks before and after are within noise
([regression check](benchmarks.md#regression-check-slice-11-general-jobs)).

**Not verified in Slice 1.1:** the manual upgrade on Windows and Linux
(CI runs P1-01 and the migration test on all three OSes); a second job
kind doing real work (extraction arrives in Slice 1.3).

## Slice 1.2

Slice 1.2 adds no screens; the status bar reads "Phase 1 slice 1.2". It
bundles PDFium on every OS and adds the extraction worker skeleton
(`atlas --extract-worker`), which must load PDFium and pass a self-test
from every installer format.

| Platform | Bundles build | PDFium in the bundle | Worker says hello | Signature |
| --- | --- | --- | --- | --- |
| macOS arm64 | ✅ CI and M5 Pro (debug and release `.app`, DMG) | ✅ `Contents/Frameworks` | ✅ CI debug and release `.app`; M5 Pro debug, release and DMG copy | ✅ `codesign --verify --deep --strict`, ad hoc with the hardened runtime and `disable-library-validation` |
| Windows x64 | ✅ CI (NSIS) | ✅ next to `atlas.exe` | ✅ CI debug build and a silent per-user install | unsigned (as before) |
| Linux x64 | ✅ CI (`.deb`, AppImage) | ✅ `/usr/lib/Atlas/` | ✅ CI debug build, unpacked `.deb`, AppImage | unsigned (as before) |

CI: full runs on branch `slice-12`,
[42531070487](https://github.com/markcheno/Atlas/actions/runs/42531070487)
(macOS and Linux green; Windows failed in the PDFium fetch, see below) and
[42531636951](https://github.com/markcheno/Atlas/actions/runs/42531636951)
(10 of 10 jobs green, including the WebDriver smoke tests on Windows and
Linux). Start times are in [benchmarks](benchmarks.md#extraction-worker-start).

What the gate found:

- **Library validation.** The first signed debug `.app` refused PDFium:
  "mapping process and mapped file (non-platform) have different Team IDs".
  An ad-hoc signature has no Team ID, so the hardened runtime's library
  validation can't match the app and the library. The spec's fallback
  entitlement, `com.apple.security.cs.disable-library-validation`, fixed it
  ([packaging](packaging.md#macos)).
- **Windows `tar`.** Under `shell: bash`, CI's `tar` is Git's GNU tar,
  which reads `D:\…` as a remote host. `fetch-pdfium.mjs` now unpacks with
  relative paths.

### macOS arm64, Slice 1.2 (2026-11-08)

All on scratch data: a copy of the Slice 1.1 scratch library, under a
scratch `HOME` for the release build.

1. `just fetch-pdfium` downloaded the macOS archive (3.4 MiB), verified its
   SHA-256 and unpacked it; a second run kept it.
2. `just ci` passed: the generator tests (including the Phase 0 golden
   hash), `tests/pdfium.rs` (PDFium reads every page and marker of a text
   fixture) and `tests/extraction_worker.rs` (hello, unknown requests, a
   missing and a non-library file, stray output on descriptor 1), 3 runs of 3.
3. The release `.app` (`pnpm tauri build --bundles app,dmg`) verifies with
   `codesign --verify --deep --strict`, carries the entitlement, and has
   `Contents/Frameworks/libpdfium.dylib` and
   `Contents/Resources/licenses/pdfium/` (19 notice files).
4. Launched with the scratch `HOME`, it opened the copied library ("30,006
   files", "Atlas 0.1.0 · Phase 1 slice 1.2"), and **Rescan** finished with
   30,006 files and none missing.
5. While it ran, `vmmap` and `lsof` showed no PDFium in the app process
   (`vmmap` listed WebKit, so it could see the process's libraries).
6. Mounted the release DMG read-only, copied `Atlas.app` out, and the copy
   verified and its worker said hello (median 18 ms).

**Not verified in Slice 1.2:** a quarantined (downloaded) DMG with the new
entitlement; Gatekeeper's prompt is expected to be unchanged. Windows and
Linux installers were checked by CI only. Extraction of library files
arrives in Slice 1.3.

## Slice 1.3

Slice 1.3 reads text. Automated evidence: P1-03 … P1-14
(`tests/extraction.rs`, `tests/extraction_crash.rs`), the supervisor tests
(`tests/supervisor.rs`) and the engines' unit tests, all against the real
worker; every extraction test checks that the originals are byte-identical
afterwards. The spike corpus's 33 documents all reach their manifest's
expected status. CI on branch `slice-13`: the quick tier
[42541656317](https://github.com/markcheno/Atlas/actions/runs/42541656317)
and the full tier
[42542436470](https://github.com/markcheno/Atlas/actions/runs/42542436470),
10 of 10 jobs green (2026-11-09), so the extraction tests ran on macOS,
Windows and Linux, with the smoke tests, benchmarks and the worker gate.

### macOS arm64, Slice 1.3 (2026-11-09)

All on scratch data (a copy of the Slice 1.2 scratch library, scratch
`ATLAS_CONFIG_DIR` and `HOME`).

1. Opened the copied schema-7 library (30,006 cataloged stub files) with
   the Slice 1.3 debug build. The catalog became schema 8, an extract job
   started by itself, and a worker process appeared. The status bar read
   "Reading text · 10,620 of 30,006 files", then "Text up to date · 26,672
   of 30,006 files readable" about a minute later (the 3,334 stub "EPUBs"
   aren't zips, so they are "Couldn't read").
2. The Library showed the Text column and filter. The first build left the
   visible page at "Waiting" after reading finished: the page refreshed on
   cataloging progress only. Fixed (the refresh now watches both), with a
   test; the rebuilt app showed "✓ Text ready" and "! Couldn't read".
3. Relaunched on simulated battery (`ATLAS_SIMULATE_POWER=battery:50`) and
   added the 1k fixture with 100 KiB of text per file: "Reading text ·
   30,020 of 31,012 files · Slowed on battery". Settings read "When Atlas
   catalogs your folders or reads their text, it will run slowly because
   this computer is on battery."
4. **Pause reading text** gave "Paused reading text by you" and no file was
   read for 4 s; **Resume reading text** continued. The folder line said
   "Reading text…" while paused; it now says "Reading text paused" (fixed,
   with a test).
5. Quitting mid-reading left no `atlas --extract-worker` process.
6. The Slice 1.2 release build refused the upgraded library ("format 8;
   this version supports up to 7"); `catalog.sqlite` and `atlas.json`
   stayed byte-identical.

**Not verified in Slice 1.3:** a real iCloud or OneDrive placeholder
(P1-13 uses the test source's cloud-only flag; the worker's no-recall open
and dataless check are unit-level); Windows and Linux beyond CI.

## Slice 1.4

Slice 1.4 adds full-text search (`search_documents`); the search screen
comes in 1.5. Automated evidence: P1-15 and P1-16 (`tests/search.rs`, with
snippets, filters, paging, copies, name matches and index consistency),
P1-17 (`tests/search_eval.rs`: 40/40 committed questions in the top 5,
every one at rank 1), the FTS `integrity-check` after the kill in P1-12,
and P1-19 (`just bench-search`: p95 152 ms while reading 30,000 files on
the M5 Pro, under 250 ms). CI on branch `slice-14`: the full tier
[42550108563](https://github.com/markcheno/Atlas/actions/runs/42550108563),
10 of 10 jobs green in 8 minutes (2026-11-09), with the search tests on all
three OSes and the search benchmark on the 1k text fixture. An earlier run
with the 30k text fixture in CI was cancelled after its Windows benchmark
passed 40 minutes; the 30k measurement is now local only.

### macOS arm64, Slice 1.4 (2026-11-09)

All on scratch data (a copy of the Slice 1.3 scratch library).

1. Opened the copied schema-8 library (31,012 files, 4,152 passages read)
   with the Slice 1.4 debug build: the window appeared in 0.9 s, the
   catalog became schema 9, the index answered `marker` (109 passages),
   and reading resumed where it had stopped.
2. `just eval <questions> <data root>` on the closed library printed each
   question's rank; the catalog was byte-identical afterwards and no WAL or
   shared-memory file was created (it opens the file as immutable when the
   app is closed). A question whose file wasn't read yet correctly missed.
3. The Slice 1.3 build refused the upgraded library ("This library needs a
   newer version of Atlas"); `catalog.sqlite` and `atlas.json` stayed
   byte-identical.

**Not verified in Slice 1.4:** search from the UI (Slice 1.5); `just eval`
on a real library (the user runs it; it is read-only).

## Slice 1.5

Slice 1.5 adds the Search view (a main tab; Home's box submits to it), the
document detail panel (from a Library name or a result's Details), and the
`get_document_detail` command. Automated evidence: `tests/documents.rs`
(title, authors and pages from a read PDF and EPUB, Also at with a missing
copy, an unread file, an unknown ID), Vitest for every Search and panel
state (snippets as text and `<mark>` only, the empty states, the reading
note and live refresh, paging and capped totals, focus return, the Tab
trap), and two new smoke steps on Windows and Linux: "quit while reading
leaves no worker process" and "search finds a passage" on the 1,000-file
text fixture.

CI on branch `slice-15`: the full tier
[42553889553](https://github.com/markcheno/Atlas/actions/runs/42553889553),
10 of 10 jobs green in 9.5 minutes (2026-11-09). In both smoke runs the
worker was caught mid-reading ("Reading text · 126 of 1,006 files" on
Linux, 129 on Windows), and quitting left no `--extract-worker` process.
After the relaunch, reading finished ("Text up to date · 1,006 of 1,006
files readable", 30 s into the run on Linux and 79 s on Windows), and
`"Astronomy marker 0"` found Café crème book 000000.epub with the phrase
highlighted. The whole smoke run took 32 s on Linux and 83 s on Windows.
After the smart-quote fix, the final branch ran the full tier again:
[42554815801](https://github.com/markcheno/Atlas/actions/runs/42554815801),
10 of 10 jobs green in 8.7 minutes.

### macOS arm64, Slice 1.5 (2026-11-09)

All on scratch data: the 1,000-file text fixture (`--text-kb 4`, 1,007
files with one extra copy), a scratch data root and `ATLAS_CONFIG_DIR`,
and `ATLAS_E2E_OPENER_LOG`, so nothing was opened in another app. The
debug build ran on battery, in dark mode.

1. Onboarding, cataloging and reading finished in seconds: "Text up to
   date · 1,007 of 1,007 files readable". Home showed "Search your
   library" and the new help text.
2. Searching `"Astronomy marker 0"` from Home opened Search. **Found a
   bug:** macOS smart quotes typed the opening quote as `“`, which the query
   builder didn't treat as a quote, so the phrase became separate words
   (the right book first, then any-words results). Fixed: `“ ” „ ‟ « »` and
   the full-width `＂` now quote too (unit test
   `typographic_quotes_make_phrases_too`). After the fix it was exactly one
   document: "Astronomy Café crème book 000000", Chapter 1, part 1, the
   phrase highlighted, "In the text", "Also in 1 other place".
3. Details showed the title, file name, author Ada Lin, English, 4
   chapters, type, size, modified, folder, path, "✓ Text ready", and Also
   at with the copy, its status and its own Open and Show in Finder. The
   panel is announced as a modal dialog, and focus lands on Close.
4. Escape closed it, and Return then reopened it, so focus had gone back to
   Details.
5. Cmd+Q with an idle `--extract-worker` running left no Atlas process.

**Known issue:** EPUB snippets can start with `# Chapter 1`: html2text
writes headings as Markdown, and the stored text keeps the `#`. It's
cosmetic, and is for Slice 1.6 (text clean-up, which means reading files
again).

**Not verified on macOS in Slice 1.5:** light mode; a full keyboard-only
pass and the Tab trap in WebKit (Vitest covers the trap); filters and paging
by hand; Open from a result. The run was stopped early because the
keystrokes it sends go to whichever app is in front.

## Slice 1.6

Slice 1.6 adds titles from a PDF's first page or the file name, EPUB text
without Markdown marks, re-reading that stays searchable (ADR 0011),
Library › Duplicates (`list_duplicates`) and Settings › Storage
(`get_storage_usage`, `clean_caches`). Automated evidence:

- `tests/metadata.rs` (P1-18): metadata, first-page and file-name titles;
  cleaned junk; the six arXiv papers' titles found on their first pages.
- `tests/refresh.rs`: an upgraded library stays searchable and "Text
  ready" while EPUBs and untitled PDFs are read again; titled PDFs are not
  re-read; a refresh that keeps crashing keeps the old text.
- `tests/duplicates.rs`: groups, extra space, filters, paging, missing and
  disconnected copies.
- `tests/storage.rs`: usage by category, the estimate's three states, and
  Clean caches emptying only `cache/` and `tmp/` without following links.
- Vitest for every new screen state, and two new smoke steps on Windows and
  Linux: "duplicates list the six Introduction copies" and "settings show
  storage".

CI on branch `slice-16`: the full tier
[42562391167](https://github.com/markcheno/Atlas/actions/runs/42562391167),
10 of 10 jobs green in 10 minutes (2026-11-09). Smoke on Linux (39 s) and
Windows (107 s): "duplicates list the six Introduction copies" (one group,
6 copies, "1 group · 28.5 KB in extra copies") and "settings show storage"
(the estimate and "Nothing to clean"), alongside Slice 1.5's steps (the
worker was again caught mid-reading on both OSes and gone after quit). An
earlier run failed on Windows only because `tests/metadata.rs` imported a
Node module by a bare `D:\…` path; it now uses a `file://` URL.

### macOS arm64, Slice 1.6 (2026-11-09)

On scratch data: a copy of the Slice 1.5 scratch library (1,007 files,
1,001 documents read by the version-1 readers, 1,336 EPUB passages starting
with `# `), opened by the Slice 1.6 debug build with a scratch
`ATLAS_CONFIG_DIR`. Atlas was driven only through accessibility actions on
its own process.

1. The build offered "Open this library"; the catalog moved to schema 10
   and the re-read started at once. A second after opening, 142 documents
   were still marked and 859 were already at version 2. No available file
   was unlinked at any point; both checks of the catalog during the
   upgrade found none.
2. Afterwards: no passage starts with `#` (Café crème book 000000's first
   passage now begins "Chapter 1"), all 1,001 documents are at version 2,
   none is marked. Every PDF in that fixture has a metadata title, so no
   PDF was read again, as designed. The status bar read "Text up to date ·
   1,007 of 1,007 files readable".
3. Quitting through the Atlas menu with a worker running left no Atlas
   process.
4. The 30k search benchmark, with duplicates and storage timings, is in
   [benchmarks](benchmarks.md#duplicates-and-storage-slice-16).

**Not verified on macOS in Slice 1.6:** the re-read was too fast to see
the "Updating text" line live (Rust and Vitest cover it). Library ›
Duplicates, Settings › Storage, Clean caches and the title labels were not
checked by hand: the Atlas window moved out of reach while the Mac was in
use, so the check stopped rather than send more keys (Vitest covers every
state, and the CI smoke runs on Windows and Linux press through both
screens). Light and dark mode were not compared.

## Slice 1.7

Slice 1.7 closes the Phase 1 carry-overs: cheaper replays when a scan
resumes inside a large folder, Windows junctions and long paths, no
extraction worker outliving the app, 16 KiB pages for new catalogs, and
the extraction benchmark. Automated evidence:

- `tests/inventory.rs` P1-20: resuming inside a 4,000-file folder at a
  fixed rate reaches the next new file in under half the time a full-cost
  replay takes (it fails at 4.0 s with the discount turned off). Unit tests
  in `inventory/db.rs` cover replayed, changed and rescanned rows.
- `tests/inventory.rs` P1-21 (Windows only): a junction looping to the
  folder and one leaving it are skipped and counted; nothing behind them
  is cataloged. `tests/extraction.rs` P1-21 (all OSes): a file at a path
  of over 300 characters is cataloged, read by the worker, found by a
  search for a word from its text, and resolved for Open.
- `tests/extraction_crash.rs` P1-22: a child app is killed while its worker
  reads (P1-12's test) and while its worker hangs inside a request; the
  worker is gone within 5 s on every OS (it fails with the watchdog turned
  off), and relaunching reads the file.
- `catalog` unit tests: a new catalog has 16 KiB pages; an existing one
  keeps 4 KiB and upgrades.
- `extraction::db` checks the query plans of the re-read queries.
- `benches/extraction.rs` on the 1k text fixture on every OS (CI bench
  job), including a kill mid-read.

CI on branch `slice-17`, full tier:
[42634733190](https://github.com/markcheno/Atlas/actions/runs/42634733190)
(first commit; green after rerunning the Windows smoke test, whose first
step lost its msedgedriver connection before the window opened, the same
flake as in Slice 1.5),
[42645560955](https://github.com/markcheno/Atlas/actions/runs/42645560955)
(10 of 10 green, 13 minutes),
[42648186627](https://github.com/markcheno/Atlas/actions/runs/42648186627)
and, for the final code,
[42651988551](https://github.com/markcheno/Atlas/actions/runs/42651988551)
(the Windows smoke test hit the same msedgedriver flake at its first step
and passed on rerun). P1-20 at the policy rate in that run: 0.85 s on
Windows, 0.94 s on Linux, 0.97 s on macOS.
The Windows test log shows `p1_21_junctions_are_skipped_and_counted`,
`p1_21_a_300_character_path_is_cataloged_and_read`,
`p1_22_a_hung_worker_does_not_outlive_a_killed_app` and
`p1_20_…` passing. Junctions needed no code change: std reports them as
symlinks.

### macOS arm64, Slice 1.7 (2026-11-09)

On scratch data: a copy of the Slice 1.6 scratch library (1,007 files),
with its 334 EPUB documents set back to reader version 1 so opening it
starts a re-read; the Slice 1.7 debug bundle with a scratch
`ATLAS_CONFIG_DIR`. No keystrokes were sent; the app was killed by the
process ID the check started, and quit through its own menu item pressed
by accessibility.

1. With the worker hung on purpose (`ATLAS_WORKER_TEST_FAULT`, debug
   builds) on the first EPUB, `kill -9` of the app: the worker was gone
   0.13 s later.
2. Without the fault, `kill -9` one second into the re-read (190 documents
   still marked): the worker was gone 0.02 s later.
3. Relaunched: the remaining documents were read within about 2.5 s; all
   are at the current reader version, no passage starts with `#`, the
   three extract jobs are completed, and the catalog kept its 4 KiB pages.
   Quitting through the Atlas menu left no `--extract-worker` process.
4. The benchmarks: P1-20 at 0.95 s, the 30k text extraction and catalog
   growth in [benchmarks](benchmarks.md#text-extraction-slice-17).

**Not verified in Slice 1.7:** opening or revealing a file at a path over
260 characters through the Windows shell (CI's smoke build records the
request instead of launching an app); a new library's 16 KiB pages in the
app by hand (unit tests and the benchmark check it); pausing and resuming
inside a large folder in the app by hand (the test and the benchmark on
three OSes check it).

## Slice 1.8

The Phase 1 close-out ([review](phase1-closeout.md)): evidence and version
0.2.0. Changes: the smoke test pauses and resumes reading and opens a
search result, waits for the native driver and retries a refused session
once; the extraction benchmark counts files by status and can require
every file to be read; the opt-in evidence run; the integrity checks after
every crash test.

**30k evidence run** ([42685716386](https://github.com/markcheno/Atlas/actions/runs/42685716386),
`just ci-remote evidence`): the 30k text fixture (32,006 files, 266,598 PDF
pages) read unthrottled on each runner, every file "Text ready":

| Runner | Read s | Pages/s | Files ready | Catalog ÷ text |
| --- | ---: | ---: | ---: | ---: |
| macOS arm64 (M1, 3 vCPU) | 445 | 599 | 32,006 of 32,006 | 1.62 |
| Windows x64 (EPYC, 4 vCPU) | 1,145 | 233 | 32,006 of 32,006 | 1.62 |
| Linux x64 (EPYC, 4 vCPU) | 431 | 618 | 32,006 of 32,006 | 1.62 |

**CI on `slice-18`:** full runs
[42683848551](https://github.com/markcheno/Atlas/actions/runs/42683848551)
(10 of 10 green; the Windows smoke passed first time) and
[42689464772](https://github.com/markcheno/Atlas/actions/runs/42689464772)
(10 of 10 green, with the new smoke steps: "pause and resume reading" held
the count at 137 of 1,006 files on Linux and 170 on Windows while paused,
and "open from a search result" opened `Café crème book 000000.epub`; the
native driver's port was open before the first session on both, so no
retry was needed).

### macOS arm64, Slice 1.8: release DMG smoke (2026-11-09)

Run by the product owner from the
[checklist](#release-dmg-smoke-slice-18), with the CI-built
`Atlas_0.2.0_aarch64.dmg` (11.6 MiB, run 42683848551) started from
Terminal with a scratch `HOME`, a fresh scratch library, and a scratch
fixture of 112 files (the 1k generator at 100 files with 4 KB of text,
plus six spike-corpus files), then a 3,006-file folder for the pause step.

Every step passed: Welcome with no account or AI step; the native folder
dialogs; cataloging and reading to "Text up to date"; the Text column and
its reasons (Encrypted, Needs OCR, Couldn't read); phrase search with
highlighting; Open in Preview and Show in Finder; Duplicates and Storage;
pausing and resuming reading in the larger folder; quitting with no
`--extract-worker` left and relaunching to the same library with nothing
read again; light and dark mode.

One expectation in the checklist was wrong, not the app: unquoted,
`Astronomy marker 60` shows the two documents with all three words first
(`book 000060.epub` at the top), then fills the page with "Some of your
words" results, as decided in Slice 1.4. The product owner kept that
behaviour, and the checklist now says so.

## Slice 2.1

Model roles, capability tests, recommendations and the three-way privacy
label ([ADR 0012](adr/0012-model-roles-and-capability-tests.md)). On
every OS, the capability tests run against the scripted local service in
`src-tauri/tests/roles.rs` (P2-01, P2-02, P2-20), and the recommendations
are unit-tested (P2-03).

### macOS arm64, Slice 2.1 (2026-11-09)

Run by the product owner against the real oMLX on the reference machine
(`http://localhost:8000/v1`, with its key typed into a scratch debug Atlas
and stored only under the debug keychain service), with a scratch
`ATLAS_CONFIG_DIR` and a scratch library.

- **Embeddings.** The owner downloaded
  `mlx-community/Qwen3-Embedding-0.6B-8bit` in oMLX's Model Downloader.
  oMLX lists it as `Qwen3-Embedding-0.6B-8bit`. Test embeddings passed
  with **1,024 dimensions**, and Use saved the role. The oMLX entry in
  `recommendations.json` is now verified and uses the id oMLX lists.
- **Chat.** Test chat passed on the owner's default model,
  `Qwen3.8-27B-MLX-4bit`, and Use saved the role.
- **Wrong role, removing the service.** These passed in the first run.
- **Quit and relaunch.** In the first run, only the chat role survived.
  The embedding row had shown the model, so it had been saved and then
  removed. The likely cause was a slip on Clear, which removed a role at
  once and sits just above Chat's Change. A scripted service kept both
  roles across quit and relaunch. Clear now asks first ("Stop using this
  model for embeddings?", with Clear and Keep). In the second run, both
  roles survived, and the scratch catalog held both rows afterwards
  (embedding with 1,024 dimensions).
- **Wording.** A test that answered in under 100 ms read "answered in
  0 s"; it now reads "answered in under 0.1 s".

## Slice 2.2

Chunks, durable embeddings and the vector index
([ADR 0013](adr/0013-embeddings-durable-index-disposable.md)). There is no
user-visible change, so nothing is checked by hand. On every OS:

- usearch's C++ builds with the app;
- `tests/vectors.rs` (P2-04, P2-08) runs, including a library under the
  non-ASCII path `Bibliothèque José 図書館` through both index I/O routes;
- `tests/vectors_crash.rs` kills a saving child process eight times;
- the bench job measures the index at 100,000 vectors.

**CI on `slice-22`:** full run
[42724704393](https://github.com/markcheno/Atlas/actions/runs/42724704393),
all green after one rerun.

- On Windows, every Rust suite passed the first time. That includes the
  non-ASCII library path through both index I/O routes, and the crash test
  in 73 s.
- The first attempt failed in Vitest: "keyboard shortcuts switch views"
  ran 5.5 s against its 5 s limit, and the theme test after it failed in
  turn. The rerun of that job passed; this slice changed no frontend code
  beyond the slice string. If it recurs, give that test a longer timeout.
- The bench jobs measured the index at 100,000 vectors on all three
  runners ([benchmarks](benchmarks.md#vector-index-slice-22)).
- Installer sizes grew by 0.5 to 1.7 MiB ([packaging](packaging.md)).

### macOS arm64, Slice 2.2 (2026-11-09)

- **`just ci` passed:** 217 Rust unit tests, every integration suite (both
  vector suites included) and 120 Vitest tests.
- **Crash test:** two of the eight kills left a staged file in `tmp/`
  mid-write. The other six came after the rename, leaving an unrecorded
  newer generation. Each time the recorded generation loaded, the index
  matched the catalog exactly, and the stray file was removed. SQLite's
  and FTS5's integrity checks passed after every kill.
- **Benchmarks** on the reference machine at 100,000, 300,000 and
  1,000,000 vectors ([benchmarks](benchmarks.md#vector-index-slice-22)).
- **Build cost:** usearch, numkong and cxx add about 8 s to a clean release
  build (cargo `--timings`).

## Slice 2.3

The embedding job ([ADR 0014](adr/0014-the-embedding-job.md)). On every
OS, these run against a scripted backend and a scripted HTTP service:
`tests/embedding.rs` (P2-05, P2-06, P2-07, P2-09, P2-19, the remote opt-in),
`tests/embedding_http.rs`, `tests/embedding_crash.rs` and
`tests/convert.rs`.

**CI on `slice-23`:** full run
[42744944343](https://github.com/markcheno/Atlas/actions/runs/42744944343),
all green. The run before it,
[42742961392](https://github.com/markcheno/Atlas/actions/runs/42742961392),
failed one Windows test that expected a small index file to load after
removals. More than 20% of its slots were removed, so the correct outcome
was compaction. On macOS a concurrent test had switched the index to the
buffer I/O route, where the compaction check was skipped. The check now
reads the file header on both routes, and the test expects the compaction.

### macOS arm64, Slice 2.3 (2026-11-09)

Run by the product owner on the reference machine against the real oMLX
(Qwen3-Embedding-0.6B-8bit, 1,024 dimensions), with the debug build and a
scratch library:

- **The library:** the spike corpus plus 1,000 generated books, 1,020
  documents with text and 22.4 million characters.
- **The conversion:** after reading, the scratch catalog was rewritten with
  4 KiB pages, as a library created before Slice 1.7. **Use** on the
  embedding model converted it to 16 KiB pages before anything was
  embedded.

Every step passed:

- the status line ("Indexing meaning · X of N files") and Home's note;
- **pulling the power plug** paused it ("Paused indexing meaning · On
  battery"), and plugging back in resumed it;
- **quitting mid-way and relaunching** continued where it stopped;
- **stopping oMLX** led to retries, then "Can't index meaning · AI service
  not reachable"; starting it and pressing Retry finished the job;
- Settings › Storage showed the Meaning index and its estimate, and Clean
  caches left it.

The catalog afterwards:

- **Complete:** 1,020 of 1,020 documents embedded; the model `ready`.
- **Nothing sent twice:** 26,561 chunks (26,453 distinct texts) and 26,453
  vectors. The one embed job sent exactly 26,453 chunks across the quit,
  the outage and its two retries.
- **Throughput:** 24.8 million characters in 368 s of waiting on oMLX,
  about **72 chunks per second** ([benchmarks](benchmarks.md#embedding-on-omlx-slice-23)).
- **Size:** the index file is 58.3 MB (0.536 of the raw f32 vectors), and
  the catalog is 105.7 MB.

## Slice 2.4

Hybrid search ([ADR 0010 amendment](adr/0010-full-text-search.md#amendment-hybrid-search-phase-2-slice-24-2026-11-09)).
On every OS:

- `tests/hybrid_search.rs` (P2-10, P2-11) and `tests/semantic_eval.rs` (the
  plumbing, with the fake backend);
- the bench job's embedding benchmark at 1k.

**CI on `slice-24`:** full run
[42753958804](https://github.com/markcheno/Atlas/actions/runs/42753958804),
all green. The run before it,
[42751707645](https://github.com/markcheno/Atlas/actions/runs/42751707645),
failed `semantic_eval` on Windows: it compared a relevant path built with
`/` against a display path with `\`. The test now compares paths as paths.

### macOS arm64, Slice 2.4 (2026-11-09)

The product owner ran the quality gate (P2-17) against oMLX on the
reference machine twice:

- **Before the dimension decision:** 512 dimensions were computed in
  memory.
- **After it, on what ships:** Atlas's own 512-dimension index, with the
  0.25 similarity floor.

Both passed: hybrid hit@5 0.950 on the vague set (text only 0.875), and
Phase 1's set 40/40 ([benchmarks](benchmarks.md#meaning-search-slice-24)).
P2-18 ran locally on the 30k text fixture: hybrid search p95 228 ms while embedding 733,668 chunks, under the 250 ms target.

## Slice 2.5

Related documents ([ADR 0013 amendment](adr/0013-embeddings-durable-index-disposable.md#amendment-related-documents-phase-2-slice-25-2026-11-10)).
On every OS:

- `tests/related.rs` (P2-12, with the fake backend);
- the Vitest cases for the Related section and More like this;
- related latency in the bench job's embedding benchmark at 1k.

**CI on `slice-25`:** full run
[42837639498](https://github.com/markcheno/Atlas/actions/runs/42837639498),
all green. Related at 1k on the CI runners, p50 / p95 (cold), ten related
per document: macOS arm64 3.3 / 3.8 ms, Linux x64 4.1 / 4.5 ms, Windows
x64 6.8 / 8.1 ms.

## Slice 2.6

The Ask Atlas engine and the privacy choices
([ADR 0015](adr/0015-ask-atlas-answers-citations-injection.md)). On
every OS:

- `tests/answers.rs` (P2-13 to P2-16 against a scripted chat service
  streaming SSE and NDJSON);
- unit tests for the citation parser, prompt escaping, the thinking filter,
  the stream parser and the privacy gate;
- the Vitest cases for Settings › Privacy.

**CI on `slice-26`:** full run
[42848628789](https://github.com/markcheno/Atlas/actions/runs/42848628789),
all green; `tests/answers.rs` passed 11 of 11 on each OS.

## Slice 2.6.1

Fixes from the first `just eval-ask`: thinking off, `no_answer`, the
citation rule, the Gutenberg cut, the weak test. On every OS,
`tests/answers.rs` gains an answer that only thinks, and the thinking
switch in Ollama, nearby and internet request bodies. The Gutenberg cut
has unit tests.

**CI on `slice-261`:** full run
[42882126056](https://github.com/markcheno/Atlas/actions/runs/42882126056),
all green; `tests/answers.rs` passed 12 of 12 on each OS.

## Slice 2.6.2

Fixes from the second `just eval-ask`: the prompt names a described work,
each document gives a second place by meaning, and there are 12
off-library questions for the evaluation. On every OS:

- `tests/answers.rs` gains a long document whose far end is sent as a
  second excerpt (13 tests);
- `scripts/evaluation-questions.test.mjs` checks `questions-off.json`.

**CI on `slice-262`:** full run
[42890183602](https://github.com/markcheno/Atlas/actions/runs/42890183602),
all green; `tests/answers.rs` passed 13 of 13 on each OS.

## Slice 2.6.3

From the third `just eval-ask`: the weak margin is 0.20, and a refusal is
one uncited sentence. Ask Atlas's tests run unchanged on every OS.

**CI on `slice-263`:** full run
[42895340978](https://github.com/markcheno/Atlas/actions/runs/42895340978),
all green; `tests/answers.rs` passed 13 of 13 on each OS.

## Slice 2.7

The Ask Atlas screen. On every OS:

- 19 new Vitest cases for the screen's states, and 5 for `answerText.ts`
  (179 in all);
- five new smoke steps on Windows and Linux, against `e2e/fake-ai.mjs`:
  - "set up the scripted AI service";
  - "choose the embedding and chat models";
  - "index meaning";
  - "a vague question finds the document by meaning";
  - "ask Atlas answers with a citation that opens the source".

**Fixed before the merge:**
- **Clicks under the status bar:** the Linux smoke's clicks failed on
  buttons the fixed status bar covered. Clicks now scroll their element to
  the middle first.
- **The index's first vector:** meaning search on the Linux runner missed
  it. A small index (up to 20,000 vectors) is now searched exactly
  ([ADR 0013](adr/0013-embeddings-durable-index-disposable.md), amendment).
  `ATLAS_E2E_TRACE_MEANING`'s trace showed the gap: exact 0.537, graph
  0.224.

**CI on `slice-27`:** full run
[42927865867](https://github.com/markcheno/Atlas/actions/runs/42927865867),
all green; the smoke passed every step on Windows and Linux, the Ask steps
included.

**A Windows flake seen once:** run 42908761627 failed
`related::related_documents_report_their_state_and_never_fail`. Its
`models::serving` found no model just after the test library was indexed.
It passed before and after; the cause isn't known.

## Slice 2.8

The Phase 2 close-out ([review](phase2-closeout.md)).

**Evidence run**
[42909721037](https://github.com/markcheno/Atlas/actions/runs/42909721037)
(`just ci-remote evidence`). The macOS job's first attempt couldn't reach
crates.io; its rerun passed.

| Runner | 30k text read | Files ready | 10k embedded (scripted) | Chunks/s | Hybrid p95 while embedding | Bytes per vector |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| macOS | 393 s | 32,006 of 32,006 | 3,362 s | 73 | 254 ms | 2,600 |
| Windows | 1,080 s | 32,006 of 32,006 | 2,418 s | 102 | 272 ms | 2,594 |
| Linux | 397 s | 32,006 of 32,006 | 1,208 s | 204 | 143 ms | 2,599 |

### macOS arm64, Slice 2.8: release DMG smoke (2026-11-10)

Run by the product owner from the
[checklist](#release-dmg-smoke-slice-28):
- **The build:** `Atlas_0.3.0_aarch64.dmg` from CI run
  [42930574567](https://github.com/markcheno/Atlas/actions/runs/42930574567),
  13.5 MB, with the small-index exact search.
- **Started** from Terminal with a scratch `HOME` (`env -i`, outside the
  shell's hooks).
- **The library:** a fresh scratch library, with the 100-file generated
  fixture plus the spike corpus's five books and eight papers.
- **The services:** oMLX with Qwen3-Embedding-0.6B-8bit and the chat
  model.

**All 11 steps completed, with no problems reported:**
1. Phase 1 without any AI service;
2. the service found and tested, and the Recommendation card;
3. both models chosen;
4. indexing meaning to "Meaning up to date", and Storage's Meaning index;
5. a vague question finding Alice by meaning, and Related;
6. Ask Atlas: chips, the cited passage, Stop, an off-library question,
   VoiceOver;
7. Disable all external AI;
8. the model switch;
9. Cmd+Q with no worker left, and the relaunch;
10. light and dark;
11. clean-up.

**The checklist's setup** was rewritten as exact commands during the run
(the fixture folders, `env -i` for the scratch `HOME`).

**CI on `slice-28b`** (Slice 2.8 rebased onto `main` after Slice 2.7's
merge): full run
[42943519172](https://github.com/markcheno/Atlas/actions/runs/42943519172),
all green on three OSes. The smoke passed every step on Windows and Linux.

## Slice 4.1

Diagnostics and the carry-overs (phase4-spec §15; [ADR 0021](adr/0021-diagnostics.md)).

**Verified locally (macOS arm64, M5 Pro):**
- `just ci`: 49 Rust suites and 233 Vitest tests passed.
- The new tests:
  - P4-01 (`tests/diagnostics.rs`): a session logged in order, a pause
    logged once, a closed library writing nothing, an unwritable log never
    stopping a rescan;
  - P4-02 (`tests/recording_privacy.rs`): no marker word, file name, model
    or service name, address or path in the log or a saved report;
  - P4-03: the report's counts against SQL, and a report saved with the
    knowledge store damaged;
  - P4-04 (`tests/supervisor.rs`, `tests/refresh.rs`,
    `tests/diagnostics.rs`): a worker of another protocol refused and never
    restarted; the corpus's three scanned PDFs re-read once for page counts
    4, 3 and 3, and nothing else re-read;
  - the 0015 migration test, and Clean caches keeping the log.
- The cost of recording: `cargo test --release --test diagnostics --
  --ignored --nocapture` ([benchmarks](benchmarks.md#diagnostics-slice-41)).

**The smoke** (Windows and Linux) gains "a diagnostics report is saved":
- Settings › Diagnostics › Save writes one report under the data root's
  `exports/diagnostics`.
- It reads back as report version 1 with passages counted.
- It holds none of the session's question, the fixture's file names, or
  the scratch, library or data-root paths.
- Show in… reveals it (the opener log).

**CI on `slice-41`:** to be recorded.

### The owner's report from the real library

The 4.1 build migrates the catalog to version 15, and the 0.4.0 release
refuses a catalog that new. Copy the catalog first, so you can go back.
The trails and notes aren't migrated. The coding agent never opens the
library: the report you share is the evidence.

1. Quit Atlas (Cmd+Q).
2. In Finder, open the library's folder (Settings › Storage shows where).
   Copy `library/catalog.sqlite` somewhere outside it, for example to
   `~/Desktop/catalog-0.4.0.sqlite`. If `catalog.sqlite-wal` or
   `catalog.sqlite-shm` are there, Atlas didn't quit cleanly: open and
   quit 0.4.0 once more first.
3. Open `Atlas_0.4.0_aarch64.dmg` from the `slice-41` full CI run's
   artifacts and run that Atlas. Its version still says 0.4.0; the status
   bar says Phase 4 slice 4.1.
4. Wait while the status line says "Updating text": PDFs marked Needs OCR
   are read once more to count their pages.
5. Settings › Diagnostics › **Save a diagnostics report**, then
   **Show in Finder**.
6. Read the report (a JSON file), and share it if you are content to.
   These numbers size Slices 4.3 to 4.5:
   - `library.filesByFormat`;
   - `library.documentsByState`;
   - `library.ocr`: PDFs needing OCR, their pages, and empty pages in
     read PDFs;
   - `storage`;
   - `library.passages` and `library.chunks`.
7. **To go back to 0.4.0:** quit Atlas. Put the copied file back as
   `library/catalog.sqlite`, and delete any `catalog.sqlite-wal` and
   `catalog.sqlite-shm` beside it. Open 0.4.0. The `logs/` and
   `exports/diagnostics/` folders can stay; 0.4.0 ignores them.

**Result:** to be recorded when the owner reports it.

## Slice 3.9

The Phase 3 close-out ([review](phase3-closeout.md)).

**Evidence run**
[43140187034](https://github.com/markcheno/Atlas/actions/runs/43140187034)
(`just ci-remote evidence`, now Phase 3's): the knowledge store at 10,000
trails and 100,000 steps, loaded through Import, and the page view, on
every OS. All three passed at the first attempt. The numbers are in the
[Phase 3 summary](benchmarks.md#phase-3-summary-slice-39).

| Runner | Import apply s | Import peak MiB | Trails list p95 ms | 50-step trail p95 ms | Snapshot s | Page render p95 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| macOS | 22.2 | 578 | 1.1 | 48.3 | 5.10 | 30.9 |
| Windows | 46.1 | 571 | 5.3 | 65.0 | 3.55 | 58.4 |
| Linux | 19.9 | 569 | 1.8 | 44.6 | 4.37 | 30.8 |

**New on every OS:**
- **`tests/gutenberg_reread.rs`** (§13's carry-over): 12 of 788 chunks
  embedded again after the Gutenberg re-read.
- **P3-23's link cases are now required on Windows**
  (`ATLAS_TEST_REQUIRE_SYMLINKS=1` in CI), and they passed in run
  [43138111316](https://github.com/markcheno/Atlas/actions/runs/43138111316).
  Slice 3.8 couldn't say whether they ran there.
- **The bad-file test** refuses a sparse file of 1 GiB + 1 byte.

**CI on `slice-39`:**
- **Full run**
  [43138111316](https://github.com/markcheno/Atlas/actions/runs/43138111316):
  all green on three OSes, smoke included. It built the 0.4.0 bundles the
  DMG smoke used.
- **After the DMG smoke's fixes, full run**
  [43150017832](https://github.com/markcheno/Atlas/actions/runs/43150017832):
  all green on three OSes. The smoke's page view step now requires a drawn
  outline, and found it in both webviews: 439 × 18 px in WebKitGTK and
  430 × 17 px in WebView2, on `book 000457.pdf`'s first page.

### macOS arm64, Slice 3.9: release DMG smoke

<!-- dmg-smoke-39 -->
Run by the product owner from the [checklist](#release-dmg-smoke-slice-39),
2026-11-12:
- **The build:** `Atlas_0.4.0_aarch64.dmg` from CI run
  [43138111316](https://github.com/markcheno/Atlas/actions/runs/43138111316),
  13.8 MiB.
- **Started** with a scratch `HOME` and a scratch library, against oMLX.

**What was reported**, and what came of it:
1. **On battery, indexing meaning paused and couldn't be resumed.**
   - That is by design: bulk embedding waits for power.
   - The line said only "Paused indexing meaning · On battery", with no
     Resume, which read as stuck.
   - It now says "Paused indexing meaning on battery · Resumes when plugged
     in". Plugged in again, it carried on as expected.
2. **Show the page opened the cited page with no outline.** The view
   didn't say the passage wasn't found, so it was found but not drawn.
   - **The cause:** the outline was boxes positioned with inline styles.
     Tauri adds a nonce to the CSP's `style-src`, and WebKit then ignores
     `'unsafe-inline'` and blocks every `style` attribute.
   - **Vitest couldn't see it:** jsdom applies no CSP.
   - **Now:** the outline is an SVG over the image. `src/lib/csp.test.ts`
     fails on any inline style in a component, and the smoke's page view
     step requires a drawn outline in WebView2 and WebKitGTK.
   - **Also found:** the search tried only the passage's start, which a
     word hyphenated at a line end defeats. It now tries the passage in
     pieces, and the benchmark outlines 37 of 37 corpus pages (33 before).
3. **Moving the cited PDF and rescanning seemed not to work.**
   - On a second look, it did.
   - It turned up a real bug: coming back to Trails with a trail already
     open showed its sources as they were before a rescan. Trails now
     fetches the open trail again whenever it is shown (Vitest).
4. **Import seemed not to work.**
   - On a second look, it did.
   - What wasn't clear was how to export everything, and where the export
     went. Settings now says **Export everything…**, and the dialog says
     what each format is for. After an export, **Show in Finder** reveals
     the folder (`reveal_export`: Rust knows the folder).

No other problems were reported. These fixes came after this build: they
are covered by the tests above and CI's smoke on Windows and Linux, not by
a second DMG run.

## Slice 3.8

Export and import. On every OS:
- **P3-22** (`tests/export.rs`, a store built through the real services:
  a search, a question answered by the scripted service, an open, a note
  and a step's note; a second trail of the same name; a bookmark with a
  note; an answer and a passage finding; an interest and a project named
  `CON`; a "Not for this"):
  - everything as Markdown in both link styles: 7 notes, each starting
    with front matter that names it, and every link pointing at a written
    file;
  - names: forbidden characters, `CON` as `CON_`, the same name twice as
    ` 2`, a 200-character title cut to 100;
  - the trail note as recorded, its sources with their files as text; a
    finding linked back to its trail; a project with its goal and
    documents;
  - a second export where the user keeps their own note under a trail's
    name: their file is left byte for byte, ours goes beside it, a third
    export replaces only Atlas's notes, and a rename leaves the old note
    (nothing deleted);
  - a deleted book: its bookmark note says no file has it now.
- **P3-23:**
  - refused: a library folder, a folder inside it, the data root, a folder
    inside it; an `Atlas` that is a file, a link, or has a linked
    `Trails`;
  - `exports` inside the data root is allowed, as is a folder that
    contains the library;
  - a link planted at a note's name: the note goes beside it, and what it
    points at is untouched;
  - after every case, the scratch tree outside `<chosen>/Atlas` is
    compared file by file (the data root's own databases aside, and no
    `Atlas` appears in it).
  - On Windows the link cases run where the account may make symbolic
    links.
- **P3-24:**
  - everything as JSON, with no paths in it, imported into an empty
    library: every table equal;
  - into the same library: everything already here, and the store's change
    count unchanged;
  - a newer rename, note and step note win, both ways, with no step
    duplicated; the older file then changes nothing;
  - a bookmark at the same place under another id is the same bookmark;
  - a token applies once, an unknown one is expired, and applying takes a
    snapshot;
  - eight bad files, refused whole in a full store and an empty one, with
    nothing written: truncated, version 99, a bad id, a 4,001-character
    note, a payload of another version, an unknown step kind, a link to
    nothing, an unknown field;
  - a 256 MiB + 1 file (sparse) is refused as too large.
- **A generated store** (`scripts/generate-knowledge.mjs`, 950 trails)
  imports cleanly, and its dry run counts the trails expiry would remove.

**Unit tests** cover safe names, front-matter ids, links in both styles,
YAML quoting, the bad-file kinds, UTC times, the one-time token, migration
2 on a version-1 store, and `lastFolder` surviving the renderer's settings.

**Vitest** covers:
- Export… from a trail, a finding (not a bookmark), a collection and
  Settings, with the scope, format and link style sent;
- link style hidden for JSON;
- a cancelled dialog, the result sentence, a refusal's reason, and Escape
  returning focus;
- Import: a cancelled dialog, the dry run's sentence (new, here, newer,
  expiring), Cancel, Import and its report, and a refused file.

**The smoke** (Windows and Linux) gains §14's Phase 3 steps:
1. a search and a question are recorded;
2. quit and relaunch (the existing step);
3. the trail is resumed and its citation opens;
4. a bookmark is saved;
5. the page view shows a page;
6. a trail is exported to a scratch folder, its note checked on disk.

**CI on `slice-38`:** full run
[43134406571](https://github.com/markcheno/Atlas/actions/runs/43134406571),
all green on three OSes, all six new smoke steps included on Windows and
Linux (the Windows job took 14 minutes).
- **The first run**
  ([43132767095](https://github.com/markcheno/Atlas/actions/runs/43132767095))
  passed five of the new steps on Linux and failed the export step: the
  Trails tab still showed the trail opened two steps before, so the smoke
  found no list to pick from. It now goes back to the list first.
- **Not shown in the log:** whether the Windows runner's account could make
  symbolic links. P3-23 prints its skip note only to the test's captured
  output, so the log doesn't say whether those cases ran.

## Slice 3.7

The PDF page view, against the real reader worker. On every OS:
- **P3-19** (`tests/reader.rs`, a library of generated PDFs plus corpus
  files):
  - page 1 with its marker phrase is a PNG of the width asked for, 2
    pages long, outlined near where the generator writes the first line
    (72 pt in, the top fifth), every rectangle inside the image, and its
    text below holding the marker;
  - text that isn't on the page, and text that is nowhere, give the image
    without an outline;
  - the scanned corpus page is an image with nothing found;
  - the last page plus one is `render_failed` (`page_out_of_range`), and
    one worker served every page.
- **P3-20:**
  - each of the corpus's malformed PDFs renders (PDFium repairs some) or
    is refused, and the next good page renders;
  - a width over 2,000 and page 0 are `invalid_render`, refused before the
    worker is asked;
  - a worker that hangs is killed within its 5 s test inactivity limit,
    and one that grows past its memory cap is killed; the next page renders
    in a new worker each time;
  - an extraction job on another folder, running throughout, reads every
    file;
  - the main process has no PDFium module loaded (macOS through dyld,
    Linux through `/proc/self/maps`; skipped on Windows);
  - the worker is killed when idle past its configured time.
- **P3-21:**
  - a second request comes from the cache, with no new render;
  - a moved and rescanned file reuses its pages by content;
  - a file only in the cloud is refused with `file_cloud_only` unless its
    page is already cached, and renders once allowed;
  - over the cap, the least recently used pages go first;
  - Clean caches empties `cache/previews`.

Vitest covers the page view on a cited passage:
- the image and its outline, the page's text, Previous and Next, the page
  field and its bounds, Fit width and Actual size;
- Bookmark this page, Escape and focus returning, and the open recorded
  with action `page`;
- the not-found note, the couldn't-show notice with Open in the default
  app, and Download and show for a cloud-only file;
- Show the page on a PDF result and a PDF bookmark, and never on an EPUB.

**CI on `slice-37`:** full run
[43121702213](https://github.com/markcheno/Atlas/actions/runs/43121702213),
all green on three OSes at the first attempt, smoke included (Windows and
Linux).

## Slice 3.6

Interests, projects and rediscovery. On every OS:
- **P3-17** (`tests/collections.rs`):
  - an interest and a project hold a trail, a bookmark, a finding and a
    document; adding again changes nothing; the page shows each, the
    document resolved with its title; membership is reported;
  - taking a bookmark out of the project leaves the bookmark;
  - deleting the trail and the finding takes them out of every
    collection; adding a deleted trail, or to a deleted collection, is
    refused;
  - deleting the interest leaves the bookmark and clears its links;
  - a project can be finished; an interest can't, nor have a goal; an
    empty name and one of 201 characters are refused; a collection keeps
    its kind; archived and finished ones aren't active, and unarchiving
    brings one back.
- **P3-18** (`tests/suggestions.rs`, on generated books with the fake
  embedding):
  - nothing to start from suggests nothing;
  - an interest's neighbours are suggested, never its own document, each
    once, with its reason;
  - the order is the same on two calls the same day, and three hours
    later;
  - a document opened in a trail is left out;
  - Not for this for one interest leaves the other free to suggest it;
  - a file deleted and rescanned is left out;
  - recent trails seed suggestions, without the search's own results, and
    their Not for this holds;
  - without an embedding role there are none;
  - the embedding backend received no request during any of it.
- **P3-13** now adds its interest through the service: nothing in it is
  written directly any more.

Vitest covers the third tab (empty state, creating an interest or a
project with its goal, the active, finished and archived sections), a
collection's page (its four sections, Remove, Archive, Edit, Mark as
finished, Delete), Add to… on a trail, a Saved item and a document (active
ones only, membership shown, add, remove, New interest… created and
added), and Home's two sections (at most six; reasons; Details recorded as
from a suggestion; Not for this; hidden when empty).

**CI on `slice-36`:** full run
[43112066505](https://github.com/markcheno/Atlas/actions/runs/43112066505),
all green on three OSes, smoke included. The first attempt passed on macOS
and Linux. On Windows, P0-21 (Phase 0's "closing stops the worker") took
2.28 s to close a library while cataloging, against its 1 s bound. Nothing
in this slice changes closing, and the test had passed on Windows in every
run since Slice 3.2. The re-run of that job passed. It is the first
sighting; if it comes back, the close path on Windows gets a look.

## Slice 3.5

Bookmarks and saved findings. On every OS:
- **P3-15** (`tests/saved.rs`, on generated books with a scripted chat
  service):
  - **Bookmarks:**
    - from a result, at the hit's passage, with the document's id and hash
      from the catalog;
    - the same place again, and the same place from the trail's step: one
      bookmark, refreshed;
    - a page and the whole document of another book: two;
    - from a citation, with the excerpt as sent;
    - after the file moves and is rescanned, the bookmark resolves at the
      new file.
  - **Findings:**
    - an answer keeps its question, the delivered text, only the cited
      source, and its step;
    - while paused, the answer on screen is still kept;
    - after a newer question, the old answer is gone but its step can be
      kept;
    - a passage;
    - a deleted step is refused, and a finding saved from it stays whole.
  - **Notes and the list:**
    - notes of 4,000 characters are saved, 4,001 are refused, and empty
      clears;
    - the list is newest first, filters by kind, text (folded) and
      document, and pages;
    - deleting takes the item out of its interest;
    - saving is never a trail step.
  - A finding serializes with both `item` and its own `kind`.
- **P3-13** now saves its bookmark and finding through the service.

Vitest covers every Bookmark and Save button and its target, a refusal
shown, Save as a finding only on a done answer, the Trails | Saved tabs,
the Saved list (items, filters, Open as from a bookmark, notes with the
count and a refusal, Delete), and Bookmarks in this document.

**CI on `slice-35`:** full run
[43106072014](https://github.com/markcheno/Atlas/actions/runs/43106072014),
all green on three OSes, smoke included, in 14 minutes.

## Slice 3.4

The Trails screen and resuming. On every OS:
- **P3-13** (`tests/resume.rs`, scripted by
  `fixtures/knowledge/resume-scenario.json`), on generated books and the
  spike corpus's `pg11.epub`, with a scripted chat service:
  - **The session:** three searches, a question answered with citations
    and one stopped, and four sources opened. Then a bookmark, a finding
    and an interest are written, and the trail is pinned.
  - **The changes:** twelve weeks pass. One opened file moves to another
    folder, one is renamed, and the cited one is deleted. `pg11.epub`,
    put back to reader version 2, is read again by version 3 and its
    passages move one place. The embedding model is switched.
  - **After the restart and a rescan:**
    - every step is as it was, in order, and the answer reads byte for
      byte;
    - the earlier unpinned trail has expired;
    - the opened sources resolve `found`, `found`, `found_elsewhere` and
      `no_file`, at the new path and the new name;
    - the bookmark, the finding and the interest are intact;
    - Search again and Ask again add two steps to the continued trail.
  - **It found a bug, fixed here:** files found by a scan while the reading
    job was re-reading books were left unread until the next launch.

Vitest covers the Trails screen:
- the empty state; the list with its pin, filter and pages;
- every step kind, the recorded answer (bold and italic, no markup, not
  live), and each source's state;
- the cited passage with its date; opens recorded from a trail;
- Continue, Search again and Ask again, including a folder that has gone;
- rename, pin, notes (with the count and a refusal), and deleting a step
  and a trail;
- a reload on a new step;
- Home's recent trails.

**CI on `slice-34`:** full run
[43100874125](https://github.com/markcheno/Atlas/actions/runs/43100874125),
all green on three OSes, smoke included, in 14 minutes.

An earlier full run
([43093081623](https://github.com/markcheno/Atlas/actions/runs/43093081623))
passed every check and test on all three, then hit a GitHub Actions
outage. Finalizing the macOS and Windows build artifacts failed five times
(an HTML error page in place of JSON), which skipped the Windows smoke. The
run then stayed "in progress" for 56 minutes and could be neither cancelled
nor re-run. A third run failed to start while GitHub reported a major
Actions outage.

## Slice 3.3

Recording trails ([ADR 0017](adr/0017-trails-and-privacy.md)). On every
OS:
- **P3-09** (`tests/recording.rs`, on a read, embedded fixture library
  with a scripted chat service):
  - a submit, page 1, a refresh, a filter change and the same submit again
    are one search step, updated by the repeat; the same search in one
    folder is a second, with the folder's name;
  - questions done, stopped, failed midway and with nothing found are four
    steps, each with its ending and the text as delivered; the cited
    source keeps its excerpt as sent, the others none;
  - a citation, a result (details then Open: one step) and Related are
    open steps anchored at the cited passage, the hit's passage and the
    whole document; the Library is none;
  - every anchor resolves `found` at once.
- **P3-10:** paused, a search, a question, an open and nothing found leave
  `meta.changes` unchanged and all succeed; a note is still written. With
  the store refusing writes (`query_only`), the search and the answer
  succeed, the failure is reported once, and the next saved step clears it.
- **P3-11** (`tests/trails.rs`, with a settable clock): steps 29 minutes
  apart share a trail and 31 minutes apart don't; New trail starts one and
  makes no empty trail; Continue joins an old one; a relaunch 10 minutes
  later continues it.
- **P3-12:** at 90 days and a minute only the unpinned trail outside a
  collection goes, at most daily; with expiry off nothing goes; bookmarks
  and findings stay.
- **P3-14:** deleting a step recounts its trail and its filter text;
  deleting the last step deletes the trail and its place in an interest; a
  finding's sources stay byte for byte; Delete all trails returns the
  count.
- **P3-16** (`tests/recording_privacy.rs`): a child process records a
  session with marker words in the search, the question, the model's
  answer, a note and a title. None is in its stdout or stderr, in any file
  of the library or its config outside the store, or in any catalog table
  but the documents' own; the store holds all of them.
- **P3-05** now kills the writer during recorder steps too.

Vitest covers the indicator and its toggle, a failure shown once, New
trail, Home's note, the Privacy part with its confirmation naming the
count, the search intents, and opens recorded from results and citations
but not the Library.

**CI on `slice-33`:** full run
[43082397519](https://github.com/markcheno/Atlas/actions/runs/43082397519),
all green on three OSes, smoke included, in 13 minutes.

The first full run
([43078984746](https://github.com/markcheno/Atlas/actions/runs/43078984746))
passed on macOS and Linux. On Windows, P3-16 named the store itself as a
leak: the test told the store from other files by comparing path text, and
Windows paths mix `\` and `/`. It now compares path components.

## Slice 3.2

The knowledge store ([ADR 0016](adr/0016-the-knowledge-store.md)). On
every OS:
- **P3-04** (`tests/knowledge.rs`): created on first open at
  `user_version` 1 in WAL mode, and reopened as it was. A store at version
  999, a file that isn't SQLite, and someone else's SQLite file each leave
  the library open, are reported (`incompatible`, `damaged`), refuse every
  use with `knowledge_unavailable`, and keep their exact bytes.
- **P3-05** (`tests/knowledge_crash.rs`): six kills of a child writing
  trails with three steps, bookmarks and collections with an item. Only
  whole rows remain, every row was counted in `meta.changes`, and both
  databases pass `integrity_check` (the catalog's FTS check too).
- **P3-06:**
  - **The rule** (`tests/knowledge.rs`, with `TestClock`): taken when
    changed; not when unchanged; not within 24 hours; taken at the next
    open after a day. The fifteenth removes the oldest only after it is
    in place.
  - **A failed check** (unit test): the bad copy is removed and reported.
  - **Kills** (`tests/knowledge_crash.rs`): six kills of a child taking
    snapshots in a loop. Only whole snapshots that pass their check remain
    (at most 15, see ADR 0016), and the next open empties the staging
    folder.
- **P3-07** (`tests/storage.rs`): the store and its snapshots are measured.
  Clean caches leaves both byte-identical. On a nearly full drive there is
  no snapshot (`low_space`) and none is deleted.
- **P3-08** (`tests/knowledge.rs`, the real worker on a generated PDF):
  - moved, renamed, and only a copy in another folder: `found`, at the new
    file;
  - the catalog rebuilt with another id: `found` by hash;
  - unknown content: `unknown`;
  - an excerpt that is nowhere: the saved place, `excerptFound: false`;
  - passages re-split: `found_elsewhere`;
  - the bytes changed, and the folder removed: `no_file`.

  Every resolution leaves the connection's `total_changes` as it was.

**CI on `slice-32`:** full run
[43064736392](https://github.com/markcheno/Atlas/actions/runs/43064736392),
all green on three OSes, smoke included, in 15 minutes (benchmarks now
run after the merge, on `main`). The Linux mirror had recovered by then;
the fallback wasn't needed.

The first full run
([43034985393](https://github.com/markcheno/Atlas/actions/runs/43034985393))
passed every macOS and Windows job. Its Linux jobs failed three times in
setup, before any build: the runners' own Ubuntu mirror,
`azure.archive.ubuntu.com`, couldn't be resolved, and `apt-get` retried for
the job's hour. The shared setup action now bounds that first install to
5 minutes, then switches to `archive.ubuntu.com` and tries again (10
minutes). To shorten the wait before each merge, benchmarks now run only on
pushes to `main` and tags, and the jobs' timeouts are tighter: checks and
tests 30 minutes (was 75), benchmarks 30 (60), release bundles 20 (60).

Locally, one full `just ci` run failed `p2_12_related_documents…`
(`tests/related.rs:96`, the near document's related list missing the
sky), then passed six times on its own and in the next full run. This is
the second sighting of a `related` failure under load (the first was on
Windows in Phase 2); still unexplained, and unrelated to the store.

## Slice 3.1.1

Hybrid fusion's best meaning match counts twice
([ADR 0010](adr/0010-full-text-search.md), amendment). It runs on every OS
through `search::hybrid`'s unit tests (a new one: the best meaning match
leads the partial matches, below every-word matches) and the existing
hybrid, Related, Ask and fake-backend P2-17 tests.

**CI on `slice-311`:** full run
[42954946415](https://github.com/markcheno/Atlas/actions/runs/42954946415),
all green on three OSes, smoke included. The product owner's
`just eval-semantic` afterwards: hybrid hit@5 0.975 at 512 and 1,024
([benchmarks](benchmarks.md#meaning-search-slice-24)).

## Slice 3.1

The Phase 3 carry-overs ([spec](phase3-spec.md) §7.6). On every OS:
- **P3-01** (`tests/extraction.rs`): a file skipped while only in the
  cloud, then stored locally with the same size and time, is read at the
  next rescan, as is one evicted between the scan and the read. One still
  in the cloud is never opened.
- **P3-02** (unit tests): the weak margin from the model's profile; the
  default for others.
- **P3-03** (`tests/refresh.rs` and `engine/gutenberg.rs`): the corpus's
  Project Gutenberg EPUBs without their header and licence, and only those
  books read again on upgrading from reader version 2.

**CI on `slice-31`:** full run
[42951386736](https://github.com/markcheno/Atlas/actions/runs/42951386736),
all green on three OSes, smoke included. The product owner re-ran
`just eval-semantic` and `just eval-ask`
([benchmarks](benchmarks.md#meaning-search-slice-24)): hybrid hit@5 at 512
is 0.900, from 0.950; the licences had been giving two questions text
matches.

## Manual checklist for Windows and Linux

CI runs an automated smoke test on Windows and Linux (Slice 0.8, below),
but it can't exercise native folder dialogs, opening files in the OS
default app, the OS keychain, removable drives, or real batteries. This
checklist covers those parts. Run it on a real machine with a CI-built
installer or a local build, and record the OS version, architecture, and
outcome of each step. Debug builds should use a scratch `ATLAS_CONFIG_DIR`.

### Release DMG smoke (Slice 1.8)

The Phase 1 close-out's macOS check of the CI-built release DMG, with
PDFium inside. Use scratch folders: a release build ignores
`ATLAS_CONFIG_DIR`, so start it from Terminal with `HOME=<scratch>` to keep
its settings out of your own. Fixture: `just fixtures <scratch>/Books 100 0
4` plus a few spike-corpus files (`drm.epub`, `user-password.pdf`,
`synthetic-scan.pdf`, `truncated.pdf`, `pg11.epub`, an arXiv paper) in
`Books/from-the-corpus/`.

1. Copy `Atlas.app` out of the DMG and start it with the scratch `HOME`.
   It opens on Welcome, with no account or AI step.
2. Get Started, then choose an empty scratch folder for the library in the
   native dialog. Add the `Books` folder, also in the native dialog.
3. Cataloging finishes (112 files plus the unreadable `locked` folder
   reported), then reading: the status line ends "Text up to date".
4. Library, Text column: the generated files are "Text ready".
   `drm.epub` and `user-password.pdf` are "Encrypted";
   `synthetic-scan.pdf` "Needs OCR"; `truncated.pdf` "Couldn't read"; each
   with a reason in Details.
5. Search `"Astronomy marker 60"` (quoted): one result, `book 000060.epub`,
   with the phrase highlighted. Without quotes, the documents with all
   three words come first (`book 000060.epub` at the top), followed by
   many marked "Some of your words": every generated file has a
   "<subject> marker <n>" line, and the fallback fills the page (phase1-spec
   revision notes, Slice 1.4). Search `"quiet harbour lights"`: many
   results, the phrase highlighted. Search `rabbit-hole`: Alice.
6. Open a PDF result: it opens in Preview. Show in Finder: Finder selects
   it.
7. Library › Duplicates: one group, the six `Introduction.pdf` copies.
   Settings › Storage: sizes, the estimate and Clean caches.
8. Add a second, larger folder (`just fixtures <scratch>/Big 3000 0 8`).
   While its text is being read, press Pause on the reading line: the
   counts stop. Resume: they carry on from where they stopped.
9. Quit with Cmd+Q. In Terminal, `pgrep -fl -- --extract-worker` prints
   nothing. Start it again: it opens the same library, with nothing read
   again.
10. Switch the system between light and dark mode: both are readable.

### Release DMG smoke (Slice 2.8)

The Phase 2 close-out's macOS check of the CI-built 0.3.0 DMG against
oMLX: Phase 1 without any AI service first, then the librarian.

**Setup**, run from the repository. Everything goes in `~/atlas-smoke`:

```sh
# Scratch folders: a home for Atlas's settings, and an empty folder for
# its library. A release build ignores ATLAS_CONFIG_DIR.
mkdir -p ~/atlas-smoke/home ~/atlas-smoke/Library

# A generated library: 100 files with about 4 KB of text each, including
# the six Introduction.pdf copies.
just fixtures ~/atlas-smoke/Books 100 0 4

# The spike corpus's books and papers, for the meaning and Ask steps.
mkdir -p ~/atlas-smoke/Books/from-the-corpus
cp spikes/extraction-bench/corpus/epub/*.epub \
   spikes/extraction-bench/corpus/pdf/real/*.pdf \
   ~/atlas-smoke/Books/from-the-corpus/

# The app, copied out of the mounted DMG.
cp -R /Volumes/Atlas/Atlas.app ~/atlas-smoke/

# Start it with the scratch home, outside your shell's environment (mise
# and similar shell hooks warn when HOME changes).
env -i HOME="$HOME/atlas-smoke/home" PATH=/usr/bin:/bin \
  "$HOME/atlas-smoke/Atlas.app/Contents/MacOS/atlas"
```

- **`/Volumes/Atlas`:** use the name the DMG mounts under.
- **The window:** keep the Terminal window open while you test.
- **Choices in the app:** in step 1, choose `~/atlas-smoke/Library` for the
  library and add `~/atlas-smoke/Books` as its folder.
- **The key:** the oMLX key you type goes into your login keychain under
  Atlas's release name. If it isn't saved or remembered with the scratch
  `HOME`, note it.
- **oMLX:** running, with `Qwen3-Embedding-0.6B-8bit` and your chat model.

1. **Without AI.**
   - Copy `Atlas.app` out of the DMG and start it. It opens on Welcome,
     with no account or AI step.
   - Choose a scratch library folder and add `Books`, both in native
     dialogs. The status line ends "Text up to date".
   - Search `"Astronomy marker 60"` (quoted): `book 000060.epub`,
     highlighted. Open it in Preview; Show in Finder.
   - Library › Duplicates shows the six `Introduction.pdf` copies.
   - The status line has no meaning line, and results have no "Similar
     meaning". Search › Ask Atlas, asking anything: "Ask Atlas needs a chat
     model" with **Set up AI models**.
2. **The service.**
   - Settings › AI Models: **Look for AI services on this computer** finds
     oMLX (or Add a service… › oMLX). Type the key and **Test**: it lists
     your models.
   - Before you choose a model, the Embeddings role shows a Recommendation
     card. It names a model and the oMLX steps, and downloads nothing.
3. **The models.**
   - Embeddings: `Qwen3-Embedding-0.6B-8bit`, Test: "Works for embeddings ·
     1,024 dimensions", then Use. Note which dimensions the role line
     shows; Atlas stores this model at 512.
   - Chat: your model, Test, Use.
4. **Indexing meaning.**
   - The status line goes from "Indexing meaning · n of N files" to
     "Meaning up to date · N files".
   - Settings › Storage shows a Meaning index category, and the estimate
     covers it.
   - Optional, on a laptop: unplug while it indexes. "Paused indexing
     meaning · On battery". Plug in and it carries on.
5. **Search and Related.**
   - Search `a bored child on a riverbank wanders into a strange
     underground world`: an Alice book at the top, "Similar meaning".
   - Details shows Related: other documents with where they match, never
     the same file's copies. More like this on a result opens it.
6. **Ask Atlas.** Search › Ask Atlas, `which book has a tutor who insists
   that ours is the best of all possible worlds`:
   - Waiting, then the answer with chips; it names Candide.
   - Sources used lists every passage, the cited ones marked.
   - A chip opens the details panel with Cited passage; Escape returns to
     the chip.
   - Ask again and Stop while it streams: "Stopped. The answer so far is
     kept."
   - Ask `how do I fix a flat bicycle tyre`: "Atlas found little…", and a
     one-sentence answer with no chips.
   - With VoiceOver (⌘F5): "Atlas is answering", then the answer read
     once. A chip reads "Source n: title, location".
7. **Privacy.**
   - Settings › Privacy › **Disable all external AI**: the embedding
     opt-in goes unavailable, and oMLX (this computer) still tests and
     answers. Turn it off.
   - Optional, with an internet profile as the chat model: "Send excerpts
     to an internet service?" with the real sizes. Cancel sends nothing.
8. **A model switch.**
   - If oMLX serves a second embedding model, choose it for Embeddings
     and confirm **Build a new index**. The line says "Rebuilding meaning
     index · … search uses the previous index", and step 5's search still
     finds Alice while it builds.
   - Afterwards the old model's index is gone from Storage.
   - Without a second model, write "not run".
9. **Quit.** Cmd+Q. `pgrep -fl -- --extract-worker` prints nothing. Start
   it again: the same library, "Meaning up to date", nothing read or
   embedded again.
10. **Light and dark:** both readable, the answer and its chips included.
11. **Clean up:** Settings › AI Models › Remove the oMLX service, which
    deletes its key. Then `rm -rf ~/atlas-smoke`.

### Release DMG smoke (Slice 3.9)

The Phase 3 close-out's macOS check of the CI-built 0.4.0 DMG against oMLX:
a real research session, closed, changed and resumed (phase3-spec §14).

**Setup:** as for Slice 2.8, with a fresh `~/atlas-smoke` (remove the old
one first), plus two scratch folders for export and a second library:

```sh
mkdir -p ~/atlas-smoke/home ~/atlas-smoke/Library ~/atlas-smoke/Library2 \
         ~/atlas-smoke/Vault ~/atlas-smoke/Moved
just fixtures ~/atlas-smoke/Books 100 0 4
mkdir -p ~/atlas-smoke/Books/from-the-corpus
cp spikes/extraction-bench/corpus/epub/*.epub \
   spikes/extraction-bench/corpus/pdf/real/*.pdf \
   ~/atlas-smoke/Books/from-the-corpus/
cp -R /Volumes/Atlas/Atlas.app ~/atlas-smoke/
env -i HOME="$HOME/atlas-smoke/home" PATH=/usr/bin:/bin \
  "$HOME/atlas-smoke/Atlas.app/Contents/MacOS/atlas"
```

Choose `~/atlas-smoke/Library` for the library and add
`~/atlas-smoke/Books`. Keep the Terminal window open.

1. **Recording, without AI.**
   - Before any search, Trails says what trails are.
   - Search `"Engineering marker 1"` (quoted), then `harbour`. The status
     bar shows that the trail is recording. Trails lists one trail with
     both searches.
   - **Pause recording trails** (status bar) and search `orbital`: nothing
     new in the trail. **Resume recording trails**.
2. **oMLX and both models,** as in Slice 2.8's steps 2 and 3; then
   "Meaning up to date".
3. **A research session.**
   - Ask a question whose sources include a PDF, for example about one of
     the corpus papers. Open a citation: the details panel shows the cited
     passage.
   - **Show the page** on that PDF citation, or on a PDF search result:
     - the page, with the passage outlined when found, and its text below;
     - Next and Previous, the page field, Fit width and Actual size;
     - **Bookmark this page**;
     - Escape returns to the button.
   - On a result, **Bookmark**. On the answer, **Save as a finding**. Give
     the finding a note in Trails › Saved.
   - Trails › Interests and projects: **New interest…** and **New
     project…** (with a goal). Use **Add to…** on the trail, the bookmark
     and the finding.
4. **Changes while it's closed.**
   - Note which PDF the answer cited.
   - Cmd+Q. In Finder, move that PDF into `~/atlas-smoke/Moved`, then
     move `Moved` into `Books`. Delete one other book the trail shows,
     for example a search result.
   - Start Atlas again, then Settings › Library Folders › **Rescan**.
5. **Resuming.**
   - Trails › the session: every step in order, and the answer word for
     word as it was.
   - The moved PDF's citation opens it at its place; Show the page still
     works.
   - The deleted book says it's no longer in the library, and still shows
     the passage that was seen.
   - **Continue this trail**, then **Search again** and **Ask again**: the
     new steps join the same trail.
6. **Home:** Pick up where you left off, the interest and project, and up to
   three suggestions with their reason. **Not for this** removes one.
7. **Export.**
   - First put a note of your own at
     `~/atlas-smoke/Vault/Atlas/Trails/<date> <trail's name>.md`, with any
     text. Use the trail's name as Trails shows it and today's date in UTC,
     for example `2026-11-12 harbour.md`.
   - On the trail, **Export…** › Markdown notes › Obsidian links › choose
     `~/atlas-smoke/Vault`:
     - the result says where it wrote, and that 1 already existed and was
       left alone;
     - your note is unchanged, and Atlas's sits beside it as "… 2.md";
     - open both in a text editor (or Obsidian).
   - Settings › Export and import › **Export everything…** › Markdown
     links, into the same Vault. Notes there link to each other.
     **Show in Finder** opens the folder Atlas wrote to.
   - **Export…** › Everything › One JSON file, into the Vault.
8. **Import into a second library.**
   - Settings › Storage › **Use a different location…** ›
     `~/atlas-smoke/Library2`, and add the same Books folder.
   - Settings › Export and import › **Import from a file…** › the JSON:
     - the sentence says how many trails, findings, bookmarks, and
       interests and projects are new, and that importing never removes
       anything;
     - **Import**: Trails, Saved and Interests and projects show them.
   - Import the same file again: everything is already here.
9. **Your data is kept.**
   - Settings › Storage shows Your trails and notes and Snapshots (at least
     one, from the import). **Clean caches**: trails, bookmarks and findings
     all stay.
   - Settings › Privacy › **Delete all trails…**: the confirmation names
     the count. Bookmarks and findings stay.
10. **VoiceOver** (⌘F5) on the page view (it reads "Page n of N" and the
    page's text) and on a trail's timeline. Then light and dark.
11. **Optional, with iCloud Drive:** a book offloaded to iCloud ("Remove
    Download"), downloaded again, is read at the next rescan.
12. **Clean up:** remove the oMLX service in Settings › AI Models, then
    `rm -rf ~/atlas-smoke`.

### Installers (Slice 0.8)

1. Download the release artifact for the OS from a green CI run.
2. Windows: run the `-setup.exe`. Confirm SmartScreen's "Windows protected
   your PC", then More info › Run anyway, and that it installs without an
   administrator prompt. Launch Atlas from the Start menu.
3. Linux: `chmod +x` and run the AppImage; then `sudo apt install ./Atlas_*.deb`
   and launch from the application menu. Record the distribution, desktop
   environment, display server (X11 or Wayland), and WebKitGTK version.
4. Uninstall (Windows: Settings › Apps; Linux: `sudo apt remove atlas`) and
   confirm the library folder you chose is left untouched.

### App shell (Slice 0.1)

Run on a clean checkout and record the OS version, architecture, and outcome
of each step.

1. Install the prerequisites listed in the README for the platform.
2. `pnpm install --frozen-lockfile`
3. `pnpm check`, which must pass with no failures.
4. `pnpm tauri dev`, then confirm a window titled "Atlas" opens on Welcome.
5. Press Tab, and confirm focus is visible. Activate Get Started with Enter or
   Space.
6. Confirm the status bar shows "Atlas 0.1.0 · Phase 0 slice 0.8", followed
   by "· development build" for debug builds only. This proves IPC works.
7. Press Ctrl+3 and confirm the Library view opens (empty until a folder is
   cataloged); Ctrl+2 opens Search (Slice 1.5).
8. Press Ctrl+4, choose Dark, then Light, then Match system. Confirm the
   theme follows, and that Match system tracks the OS setting.
9. Close the window. Confirm no `atlas` process remains (Task Manager, or
   `pgrep atlas`).
10. Windows: `pnpm tauri build --debug --bundles nsis`, then install the setup
    `.exe`, launch Atlas from the Start menu, and repeat steps 4–9. Note any
    SmartScreen prompt.
11. Linux: `pnpm tauri build --debug --bundles appimage,deb`, then run the
    AppImage and install the `.deb`, repeating steps 4–9 for each. Record the
    distribution, desktop environment, and WebKitGTK version.

### Data location (Slice 0.2)

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

### Library folders (Slice 0.3)

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

### Device check (Slice 0.4)

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

### Cataloging (Slice 0.5)

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

### Upgrade from Phase 0 (Slice 1.1)

1. With a Phase 0 build (0.1.0 from a `phase0/slice-09` CI run) and a
   scratch config folder, add a fixture of 2,000 files or more, pause the
   scan part-way, and quit.
2. Copy the library folder. Open the copy with a Slice 1.1 build. Confirm
   the scan still shows "Paused by you" with the same file count, and the
   status bar says "Phase 1 slice 1.1".
3. Resume and confirm it completes with the fixture's `catalogableFiles`
   count and no missing files.
4. Open the migrated copy with the Phase 0 build again. Confirm it shows
   "This library needs a newer version of Atlas" and the catalog file is
   unchanged.

### Extraction worker (Slice 1.2)

1. After installing, find the PDFium library: Windows `pdfium.dll` next to
   `atlas.exe` in the install folder; Linux `/usr/lib/Atlas/libpdfium.so`
   (`.deb`). Confirm `licenses/pdfium/` is beside it.
2. From a terminal, run the installed executable with `--extract-worker`.
   The first line must be `{"type":"hello",…,"selfTest":{"pages":2,…}}`.
   Type `{}` and Enter: it must answer
   `{"type":"error","reason":"unknown_request"}`. Close the input (Ctrl+D on
   Linux, Ctrl+Z then Enter on Windows) and the process must exit.
3. Start Atlas normally and confirm it opens as before.

### Reading text (Slice 1.3)

1. Add a folder made with `just fixtures <dir> 1000 0 20`. After cataloging,
   the status bar shows "Reading text · N of 1,006 files", then "Text up to
   date". Every row's Text badge is "Text ready".
2. Pause and resume reading from the status bar; confirm cataloging
   controls are unaffected.
3. Add a few scanned PDFs and a password-protected PDF; confirm "Needs OCR"
   and "Encrypted" with a reason on hover.
4. Quit while reading; confirm no `atlas` worker process remains (Task
   Manager, or `ps`).

### Library, search, open (Slice 0.6)

1. With a fixture cataloged, search `cafe` and confirm the "Café crème"
   files appear. Clear the search with Esc.
2. Filter by folder and type, toggle "Include missing files", sort by name
   and modified date, and page with Next and Previous.
3. Open a PDF: it should open in the default viewer. Then Show in
   Explorer, Finder, or the file manager: the file should be selected.
4. Delete a cataloged file and press Open. Confirm the "no longer where
   Atlas found it" message, and confirm Rescan marks it missing.
5. Windows: open an online-only OneDrive PDF. Confirm Atlas asks before
   downloading, and that the file downloads only after you confirm.
6. Linux: confirm Show in folder opens the file manager with the file
   selected (FileManager1), or at least opens its folder.

### Search and details (Slice 1.5)

With the text fixture (`--text-kb 4`) cataloged and read:

1. On Home, search `"Astronomy marker 0"` and press Enter. Confirm the
   Search view shows "Astronomy Café crème book 000000" with the phrase
   highlighted, and "In the text".
2. Search `CAFE CREME` (accents and case) and `marker 1*` (prefix); try a
   query of only punctuation and confirm "Type a word to search for.".
3. Search for words that are never together in one book and confirm the
   results say "Some of your words".
4. Filter by folder and type; page with Next and Previous.
5. Press Details on a result, and on a Library name: check the title,
   author, pages, text status and Also at (copy a file first). Press Escape
   and confirm focus returns to the button you used.
6. Open and Show in folder from a result and from the panel.
7. Only with the keyboard: Tab to the search box, search, Tab through
   results, open Details with Enter, close it with Escape.
8. Repeat 1 and 5 in dark mode.

### Titles, duplicates and storage (Slice 1.6)

With a text fixture made by `just fixtures <dir> 100 0 4 4 20` cataloged
and read:

1. Search `"Engineering marker 3"` and similar markers of files 3, 7, 11, …:
   the PDFs' results show their title with "Title from the first page" in
   Details; the EPUBs show a title from the file name.
2. Library › Duplicates (the switch in the toolbar): the six
   `Introduction.pdf` copies and the 20 `copies/` files appear as groups,
   most extra space first. Filter by folder and type; tick Include missing
   files after deleting a copy and rescanning.
3. Open and Show in folder from a copy; Details from a copy.
4. Settings › Storage: the four sizes, the estimate sentence, and Clean
   caches ("Nothing to clean", or what it freed after putting a file in the
   library's `cache/` folder).
5. Upgrade check: open a library read by Slice 1.5 with this build. EPUB
   snippets lose their `# Chapter 1`, search keeps finding them while the
   status line says "Updating text", and the files stay "Text ready".

### Carry-overs (Slice 1.7)

1. Generate the 30k fixture (`just fixtures <dir> 10000 20000`), add it,
   and pause while the status shows it in the `flat` folder. Resume: new
   files appear within about a second, not 15–20 s.
2. Windows: in a library folder, `mklink /J loop .` and a junction to a
   folder outside it. Rescan: nothing behind either is listed, and the
   scan's skipped-links count includes them.
3. Windows: put a PDF at a path over 260 characters inside a library
   folder. Rescan: it is listed, its text is read and searchable. Try Open
   and Show in folder, and note what the shell does (not tested so far).
4. While text is being read, end Atlas from Task Manager (Windows) or with
   `kill -9` (Linux): no `atlas --extract-worker` process is left. Relaunch
   and the reading finishes.

### AI services (Slice 0.7)

1. In Settings › AI Models, press "Look for AI services on this computer".
   Confirm local services are listed, and that nothing is contacted
   before you press it.
2. Add Ollama and press Test. You should see either a model list or the
   "no models installed" message.
3. Add an OpenAI-compatible service with a key and press Test. Confirm the
   key is stored (macOS: Keychain Access, service `com.altazlabs.atlas`;
   Windows: Credential Manager; Linux: Seahorse) and is not in any file
   under the Atlas library folder.
4. Enter an `http://` address on another computer. Confirm Atlas blocks it
   until you tick the acknowledgement.
5. Linux without a Secret Service (for example a minimal window manager):
   confirm the key shows as "kept for this session only".

### Model roles (Slice 2.1)

1. With Ollama running and `qwen3-embedding:0.6b` pulled, add Ollama, then
   under Models › Embeddings choose it, type or pick the model and press
   Test: "Works for embeddings · 1,024 dimensions". Press Use; the role
   shows the model and its dimensions.
2. Under Chat, choose a chat model and press Test (the first test may wait
   while the model loads): "Works for chat". Pick an embedding model for
   Chat instead and confirm Atlas says it can't chat.
3. Press Clear on a role: Atlas asks first; Keep leaves it. Quit and
   relaunch: both roles are kept.
4. Remove the service: both roles return to "Not set".
5. With no embedding model installed, the recommendation card shows the
   steps for your AI app and Atlas downloads nothing.
6. Add a service at a LAN address and one at an internet address: the
   cards say "Another computer" and "Internet service".

### Indexing meaning (Slice 2.3)

1. With Ollama running and `qwen3-embedding:0.6b` pulled, add Ollama, choose
   it under Models › Embeddings, Test, then Use. The status bar's third
   line reads "Indexing meaning · X of N files", and Home says Atlas is
   still indexing meaning.
2. Unplug the power (a laptop): "Paused indexing meaning on battery · Resumes when plugged in" (before Slice 3.9: "Paused indexing meaning · On battery"),
   while reading text, if any, only slows. Plug it back in: indexing
   resumes.
3. Quit mid-way and relaunch: indexing continues where it stopped.
4. Stop Ollama: after about a minute of retries, "Can't index meaning · AI
   service not reachable" with Retry. Start Ollama, press Retry: indexing
   finishes, "Meaning up to date · N files".
5. Settings › Storage shows "Meaning index" with its size; Clean caches
   leaves it.
6. Add a service at an internet address and choose it for Embeddings: the
   line says "Indexing meaning needs your permission" until Settings ›
   Privacy allows it.

### Searching by meaning (Slice 2.4)

1. With an embedding model chosen and the library indexed (Slice 2.3),
   search for a question in your own words, for example "a bored child on a
   riverbank wanders into a strange underground world" with the spike
   corpus. *Alice's Adventures in Wonderland* comes first, marked "Similar
   meaning" or "In the text and similar meaning".
2. Stop the AI service and search again: the same text results, with
   "Showing text matches only: the AI service isn't reachable."
3. Search for nonsense ("zqxjv plorbix"): nothing is found.

### Related documents (Slice 2.5)

1. With the library indexed for meaning, open a book's Details. Related
   lists up to ten other documents, each with "Related by meaning:" and a
   page or chapter. The book's own copies are never among them; they are
   under Also at.
2. Choose Details on a related document. The panel shows it, with "Back
   to ‹the first book›". Back returns, and Close returns focus to where
   you opened the panel.
3. Search for something, then choose More like this on a result. The
   panel opens with focus on Related.
4. Stop the AI service and open Details again. Related still shows: it
   uses only what Atlas has stored.
5. Open a file Atlas hasn't indexed yet, or a scanned PDF without text.
   Related says "Not indexed for meaning yet" or "Atlas found no text in
   this file to compare".

### Ask Atlas engine and privacy (Slice 2.6)

The Ask Atlas screen arrives in Slice 2.7. Until then:

1. With oMLX serving an embedding model and a chat model, run
   `just eval-ask` with `ATLAS_EVAL_EMBED_URL`, `ATLAS_EVAL_EMBED_MODEL`,
   `ATLAS_EVAL_EMBED_KEY` and `ATLAS_EVAL_CHAT_MODEL` set. The chat model
   uses the same service and key unless `ATLAS_EVAL_CHAT_*` say
   otherwise.
   - Every vague question gets an answer, "nothing found" or an error.
   - No `[n]` in any answer is beyond its sources.
   - The summary lines give the times, citation rates and margins for
     [benchmarks](benchmarks.md#ask-atlas-slice-26).
   - The whole answers are in `src-tauri/target/tmp/eval-ask-answers.md`.
   - It builds its own scratch library from the corpus; your library isn't
     used.

   *First run 2026-11-10 against oMLX with `Qwen3.8-27B-MLX-4bit` and
   thinking on: 40 answers, no invented citation, five empty answers (see
   benchmarks). After Slice 2.6.1, thinking off: first word p50 14.4 s,
   whole answer p50 23.4 s, no empty answers, 18 answers too literal. The
   run after Slice 2.6.2 adds the 12 off-library questions: they should
   find nothing, be weak, or be answered "not covered", citing nothing.
   Run after 2.6.2: 4 of 40 vague answers "not covered" (from 18); all 12
   off-library questions refused, 9 of them citing every source. After
   2.6.3: 8 of 12 off-library questions weak and none of the vague set; 9
   refusals cite nothing.*
2. In Settings › Privacy, turn on **Disable all external AI**. "Allow
   embedding my library through an internet service" becomes unavailable.
   Test an internet service's connection in Settings › AI Models: it says
   all external AI is turned off and sends nothing. Your oMLX service still
   tests normally. Turn it off again.

### Ask Atlas screen (Slice 2.7, macOS)

The smoke can't drive macOS's webview, so check the screen by hand with
`just dev` against oMLX and a scratch `ATLAS_CONFIG_DIR`:

1. Search › **Ask Atlas**. Ask a vague question about a book in your
   library.
   - The status says "Finding passages…", then "Waiting for the answer…".
   - The answer streams in with numbered chips, and Sources used lists
     every passage, the cited ones marked.
2. Activate a chip. The details panel opens with **Cited passage** (source
   number, page or chapter, the excerpt sent). Escape returns focus to the
   chip. Open opens the file.
3. Ask again and press **Stop** while it streams. "Stopped. The answer so
   far is kept." Ask Atlas returns.
4. Ask about something your library has nothing on. The weak note shows,
   or "Atlas found nothing…", and the answer is a single uncited sentence.
5. With VoiceOver on, ask once. It says "Atlas is answering", then reads
   the answer once at the end, not piece by piece. Each chip reads as
   "Source n: title, location".
6. Optional, with an internet profile as the chat model: the first
   question shows "Send excerpts to an internet service?" with the real
   sizes. Cancel sends nothing; Continue asks.

## Known platform notes

- **Linux:** WebKitGTK 4.1 is required at runtime. CI builds on Ubuntu 24.04,
  so its AppImage needs glibc 2.39 or newer. Older distributions are not
  claimed as supported.
- **Windows:** The NSIS installer depends on the system WebView2 runtime,
  which ships with current Windows 10 and 11.
- **Windows CI smoke:** one run in Slice 1.5 and two of four full runs in
  Slice 1.7 failed at the first step, before the app's window opened: msedgedriver refused
  tauri-driver's connection. A rerun of the job has passed every time. If it
  keeps recurring, Slice 1.8 should retry the session start once.
- **All platforms:** Bundles are unsigned and not notarized in Phase 0, so OS
  security prompts are expected.
- **Icons:** The bundles still use Tauri's default placeholder icons.
