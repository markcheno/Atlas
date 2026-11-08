# Packaging and installation (Phase 0)

Phase 0 builds are **development distributions**: unsigned, not notarized,
and not production-ready (phase0-spec §14). They exist to prove that Atlas
builds, installs, and runs on each OS. Don't hand them to people who
can't work through the security prompts described below.

## What CI builds

Every push to `main`, every `phase*/…` tag, and every manual run builds
release bundles in the `package` job of
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml). The
`check-and-build` job also builds debug bundles on every push and pull
request. Artifacts are attached to the workflow run (release bundles for 30
days, debug bundles for 14).

| OS (runner) | Formats | Artifact |
| --- | --- | --- |
| macOS arm64 (`macos-latest`) | `.app` inside a `.dmg` | `atlas-macOS-ARM64-release` |
| Windows x64 (`windows-latest`) | NSIS `-setup.exe` (per-user install) | `atlas-Windows-X64-release` |
| Linux x64 (`ubuntu-24.04`) | `.AppImage`, `.deb` | `atlas-Linux-X64-release` |

Sizes from the run recorded in [platform validation](platform-validation.md#slice-08-ci-packaging-smoke-and-benchmarks):

<!-- sizes:start -->
| Bundle | Size |
| --- | ---: |
| _filled in from CI run_ | |
<!-- sizes:end -->

Locally: `just bundle-release` (release) or `just bundle` (debug) builds
the formats for the current OS and prints their sizes.

The app needs none of Python, Docker, a database server, Ollama, or an
LLM to start. It is one executable with the UI embedded. SQLite is compiled
in, and the catalog lives in the data folder the user chooses.

## Expected security prompts

### macOS

The bundles carry only an ad-hoc signature and are not notarized. A DMG
downloaded from a browser or a CI artifact is quarantined, so the first
launch shows a dialog: *"Atlas" Not Opened. Apple could not verify "Atlas"
is free of malware…*

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
- **macOS:** uses the system WebKit; nothing extra is bundled.
