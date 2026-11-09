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
- **Windows long paths** (over 260 characters) and network shares.
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
expected status. CI: see below.

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

## Manual checklist for Windows and Linux

CI runs an automated smoke test on Windows and Linux (Slice 0.8, below),
but it can't exercise native folder dialogs, opening files in the OS
default app, the OS keychain, removable drives, or real batteries. This
checklist covers those parts. Run it on a real machine with a CI-built
installer or a local build, and record the OS version, architecture, and
outcome of each step. Debug builds should use a scratch `ATLAS_CONFIG_DIR`.

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
7. Press Ctrl+2 and confirm the Library view opens (empty until a folder is
   cataloged).
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

## Known platform notes

- **Linux:** WebKitGTK 4.1 is required at runtime. CI builds on Ubuntu 24.04,
  so its AppImage needs glibc 2.39 or newer. Older distributions are not
  claimed as supported.
- **Windows:** The NSIS installer depends on the system WebView2 runtime,
  which ships with current Windows 10 and 11.
- **All platforms:** Bundles are unsigned and not notarized in Phase 0, so OS
  security prompts are expected.
- **Icons:** The bundles still use Tauri's default placeholder icons.
