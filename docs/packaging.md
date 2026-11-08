# Packaging and installation

Phase 0 builds are **development distributions**: unsigned, not notarized,
and not production-ready (phase0-spec §14). They exist to prove that Atlas
builds, installs, and runs on each OS. Don't hand them to people who
can't work through the security prompts described below.

## What CI builds

Every push to `main`, every `phase*/…` tag, and every full manual run
(`just ci-remote`) builds release bundles in the `package` job of
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml). The
`check-and-build` job also builds debug bundles in those runs and on pull
requests. Pushes to `slice-*` branches run the quick tier, which builds no
bundles ([CI tiers](architecture.md#build-ci-and-measurement)). Artifacts are attached to the workflow run (release bundles for 30
days, debug bundles for 14).

| OS (runner) | Formats | Artifact |
| --- | --- | --- |
| macOS arm64 (`macos-latest`) | `.app` inside a `.dmg` | `atlas-macOS-ARM64-release` |
| Windows x64 (`windows-latest`) | NSIS `-setup.exe` (per-user install) | `atlas-Windows-X64-release` |
| Linux x64 (`ubuntu-24.04`) | `.AppImage`, `.deb` | `atlas-Linux-X64-release` |

| Bundle | Size | Notes |
| --- | ---: | --- |
| `Atlas_0.1.0_aarch64.dmg` | 6.8 MiB | 17 MiB app once installed |
| `Atlas_0.1.0_x64-setup.exe` | 4.7 MiB | Uses the system WebView2 |
| `Atlas_0.1.0_amd64.deb` | 8.4 MiB | Uses the distribution's WebKitGTK |
| `Atlas_0.1.0_amd64.AppImage` | 82.7 MiB | Bundles WebKitGTK, GTK and their libraries |

Release builds from CI run
[42444312761](https://github.com/markcheno/Atlas/actions/runs/42444312761)
(2026-11-08).

Locally: `just bundle-release` (release) or `just bundle` (debug) builds
the formats for the current OS and prints their sizes.

The app needs none of Python, Docker, a database server, Ollama, or an
LLM to start. It is one executable with the UI embedded. SQLite is compiled
in, and the catalog lives in the data folder the user chooses.

## PDFium in the bundles (Slice 1.2)

Every bundle ships PDFium (bblanchon/pdfium-binaries chromium/8125, pinned
with SHA-256 in `tools.lock.json`) for the extraction worker, plus its
notices in `licenses/pdfium/` (PDFium's BSD-3-Clause `pdfium.txt`, the 17
third-party notices, and `LICENSE-pdfium-binaries.txt`, the build scripts'
MIT licence). Nothing is downloaded at run time.

| OS | Where the library goes | Configured in |
| --- | --- | --- |
| macOS | `Atlas.app/Contents/Frameworks/libpdfium.dylib`, signed with the app | `src-tauri/tauri.macos.conf.json` (`bundle.macOS.frameworks`) |
| Windows | `pdfium.dll` next to `atlas.exe` in the install folder | `src-tauri/tauri.windows.conf.json` (`bundle.resources`) |
| Linux | `/usr/lib/Atlas/libpdfium.so` (`.deb`); `usr/lib/Atlas/` inside the AppImage | `src-tauri/tauri.linux.conf.json` (`bundle.resources`) |

The library is 7.0 MiB on macOS (7.2 MiB on Windows, 7.7 MiB on Linux).
The macOS release DMG grew from 6.8 to 10.4 MiB (17 to 26 MiB installed)
on the M5 Pro. CI checks that the extraction worker loads the library and
passes its self-test from every format: the signed `.app` (debug and
release), the unpacked `.deb`, the AppImage, and a silent per-user NSIS
install (`scripts/worker-hello.mjs`).

## Expected security prompts

### macOS

The app bundle is ad-hoc signed (`"signingIdentity": "-"` in
`tauri.conf.json`), with sealed resources and the hardened runtime, but no
Developer ID and no notarization. (Without the ad-hoc bundle signature,
only the binary carries the linker's signature, and macOS tends to call a
downloaded copy "damaged" with no way to open it.) A DMG downloaded with a
browser, including a CI artifact zip, is quarantined. Gatekeeper then
blocks the first launch with its standard dialog for unnotarized apps
(*"Atlas" Not Opened. Apple could not verify "Atlas" is free of malware…*).
`codesign --verify --deep --strict` passes on the CI-built bundle, and
`spctl --assess` rejects it, as expected without notarization.

Since Slice 1.2 the app is signed with one entitlement,
`com.apple.security.cs.disable-library-validation`
(`src-tauri/Entitlements.plist`). Under the hardened runtime, library
validation only lets a process load libraries signed by the same team, and
an ad-hoc signature has no Team ID, so the bundled PDFium was refused
("mapping process and mapped file (non-platform) have different Team
IDs"). With a Developer ID, the app and PDFium would share a Team ID and
the entitlement should be dropped; review it before notarization. The dialog
itself was not triggered during validation: launching through Finder
would have run Atlas against the tester's real profile.

To open it anyway:

1. Drag Atlas from the DMG to Applications.
2. Open it once. When the dialog appears, choose **Done**.
3. Open **System Settings › Privacy & Security**. Scroll to the message
   about Atlas and choose **Open Anyway**, then confirm with your password.

A build you made yourself (`just bundle-release`) isn't quarantined and
opens directly.

### Windows

The installer is unsigned, so Microsoft Defender SmartScreen shows
**Windows protected your PC**. Choose **More info › Run anyway**. The
NSIS installer installs for the current user without administrator rights.

Atlas uses the system **WebView2** runtime, which is built into
Windows 11 and current Windows 10. If it's missing, the installer's default
bootstrapper downloads it from Microsoft. That is the only network access
during installation.

### Linux

No signing prompt. Make the AppImage executable
(`chmod +x Atlas_*.AppImage`) or install the `.deb`
(`sudo apt install ./Atlas_*.deb`).

## Linux runtime dependencies

Atlas uses the system WebKitGTK webview. The `.deb` declares its
dependencies; the AppImage bundles most libraries but still expects a
working GTK and graphics stack.

| Need | Package (Debian/Ubuntu) | Notes |
| --- | --- | --- |
| Webview | `libwebkit2gtk-4.1-0` | WebKitGTK 4.1 (Ubuntu 22.04 and later) |
| Toolkit | `libgtk-3-0` | |
| Key storage (optional) | a Secret Service provider such as `gnome-keyring` or KWallet | Without one, provider keys are kept for the session only and must be re-entered after a restart |

**Tested:** Ubuntu 24.04 x64, on the GitHub runner only (build, automated
smoke under Xvfb). Other distributions, Wayland sessions, and ARM aren't
tested, and Atlas makes no claim of working on every distribution.

## Not done in Phase 0

- Code signing and notarization (Apple Developer ID, Windows Authenticode).
  Notarization must review the `disable-library-validation` entitlement
  added for PDFium in Phase 1.
- Auto-update.
- Microsoft Store, Mac App Store, Flatpak, Snap, or RPM.
- A portable single-file executable (not required, §14).
- A universal macOS binary or Intel macOS build; Windows or Linux on ARM.

## Licensing obligations of the bundles

See [dependency licenses](dependency-licenses.md). In short:

- The Rust and npm dependencies compiled into the app are under permissive
  licenses (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, and similar). The
  report flags anything else.
- **AppImage:** bundles system libraries from the build machine, including
  WebKitGTK and GTK, which are **LGPL-2.1+**. Redistributing the AppImage
  means meeting LGPL terms for those libraries: offer their source and
  allow relinking. They're unmodified Ubuntu packages, so pointing to the
  matching Ubuntu source packages is the usual way. This must be settled
  before public distribution.
- **`.deb`:** links against the distribution's WebKitGTK/GTK packages and
  bundles none of them.
- **Windows:** WebView2 is a Microsoft system component, distributed under
  Microsoft's WebView2 runtime terms by its own bootstrapper.
- **macOS:** uses the system WebKit; nothing else is bundled.
- **PDFium, every OS:** BSD-3-Clause plus permissive third-party notices
  (FreeType, libjpeg-turbo, OpenJPEG, Little CMS, libpng, zlib, Abseil,
  HarfBuzz, ICU and others). They ship in `licenses/pdfium/` in every
  bundle, which meets their notice requirements. The library is
  unmodified.
