//! Where the worker finds the PDFium library (phase1-spec §7.2).
//!
//! - Debug builds: `ATLAS_PDFIUM_LIBRARY`, when set, is the only candidate.
//! - macOS: `Contents/Frameworks/libpdfium.dylib` (`bundle.macOS.frameworks`).
//! - Windows and Linux: the Tauri resource folder (`bundle.resources`),
//!   found by the same rules as `tauri_utils::platform::resource_dir`: the
//!   executable's folder on Windows and in a Cargo `target/` folder;
//!   `../lib/Atlas`, `$APPDIR/usr/lib/Atlas` or `/usr/lib/Atlas` on Linux.
//!   They are mirrored here rather than called, because that function needs
//!   the app's `PackageInfo` and takes the executable path from the process,
//!   which tests can't change.
//! - Debug builds, last: `src-tauri/vendor/pdfium/<platform>/`, where
//!   `scripts/fetch-pdfium.mjs` unpacks it, so `cargo run` and `cargo test`
//!   need no setup.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const LIBRARY_ENV: &str = "ATLAS_PDFIUM_LIBRARY";

/// Why the worker can't extract. Never carries a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unavailable {
    /// No candidate location holds the library.
    NotFound,
    /// The file exists but is not a loadable PDFium.
    LoadFailed,
    /// PDFium loaded but could not read the built-in self-test PDF.
    SelfTestFailed,
    /// The worker speaks another protocol than this app (Slice 4.1): a
    /// mismatched install. Set by the app, never sent by a worker.
    ProtocolMismatch,
}

impl Unavailable {
    /// The job's fixed error code for this reason.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ProtocolMismatch => "worker_protocol",
            Self::NotFound | Self::LoadFailed | Self::SelfTestFailed => "extractor_unavailable",
        }
    }
}

#[cfg(target_os = "macos")]
pub const LIBRARY_FILE: &str = "libpdfium.dylib";
#[cfg(target_os = "windows")]
pub const LIBRARY_FILE: &str = "pdfium.dll";
#[cfg(all(unix, not(target_os = "macos")))]
pub const LIBRARY_FILE: &str = "libpdfium.so";

/// The product name Tauri uses for the Linux resource folder.
const PRODUCT: &str = "Atlas";

/// Inputs to the search, injectable for tests.
#[derive(Debug, Clone, Default)]
pub struct Locator {
    pub exe: PathBuf,
    /// `ATLAS_PDFIUM_LIBRARY` (debug builds only).
    pub env_override: Option<PathBuf>,
    /// `$APPDIR` of a running AppImage (Linux).
    pub appdir: Option<PathBuf>,
    /// The fetched development copy (debug builds only).
    pub dev_vendor: Option<PathBuf>,
}

impl Locator {
    pub fn from_process() -> Self {
        let debug = cfg!(debug_assertions);
        Self {
            exe: std::env::current_exe().unwrap_or_default(),
            env_override: debug
                .then(|| std::env::var_os(LIBRARY_ENV).map(PathBuf::from))
                .flatten(),
            appdir: std::env::var_os("APPDIR").map(PathBuf::from),
            dev_vendor: debug.then(dev_vendor_path).flatten(),
        }
    }

    /// Locations to try, in order.
    pub fn candidates(&self) -> Vec<PathBuf> {
        if let Some(path) = &self.env_override {
            return vec![path.clone()];
        }
        let exe_dir = self.exe.parent().unwrap_or(Path::new(""));
        let mut out = Vec::new();
        if cfg!(target_os = "macos") {
            out.push(exe_dir.join("../Frameworks").join(LIBRARY_FILE));
        } else if cfg!(target_os = "windows") || is_cargo_output_dir(exe_dir) {
            out.push(exe_dir.join(LIBRARY_FILE));
        } else {
            out.push(exe_dir.join("../lib").join(PRODUCT).join(LIBRARY_FILE));
            if let Some(appdir) = &self.appdir {
                out.push(appdir.join("usr/lib").join(PRODUCT).join(LIBRARY_FILE));
            }
            out.push(Path::new("/usr/lib").join(PRODUCT).join(LIBRARY_FILE));
        }
        out.extend(self.dev_vendor.clone());
        out
    }

    pub fn find(&self) -> Result<PathBuf, Unavailable> {
        self.candidates()
            .into_iter()
            .find(|p| p.is_file())
            .ok_or(Unavailable::NotFound)
    }
}

/// Tauri's test for an executable inside `target/<profile>` (or
/// `target/<triple>/<profile>`).
fn is_cargo_output_dir(dir: &Path) -> bool {
    let parent_is_target =
        |d: Option<&Path>| d.and_then(Path::file_name) == Some("target".as_ref());
    (parent_is_target(dir.parent()) || parent_is_target(dir.parent().and_then(Path::parent)))
        && dir.join(".cargo-lock").exists()
}

fn dev_vendor_path() -> Option<PathBuf> {
    let (platform, rel) = if cfg!(target_os = "macos") {
        ("darwin-arm64", "lib/libpdfium.dylib")
    } else if cfg!(target_os = "windows") {
        ("win32-x64", "bin/pdfium.dll")
    } else {
        ("linux-x64", "lib/libpdfium.so")
    };
    Some(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("vendor/pdfium")
            .join(platform)
            .join(rel),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    #[test]
    fn the_override_is_the_only_candidate() {
        let tmp = tempfile::tempdir().unwrap();
        let vendor = tmp.path().join("vendor").join(LIBRARY_FILE);
        touch(&vendor);
        let missing = tmp.path().join("nope").join(LIBRARY_FILE);
        let l = Locator {
            exe: tmp.path().join("bin/atlas"),
            env_override: Some(missing.clone()),
            dev_vendor: Some(vendor),
            ..Locator::default()
        };
        assert_eq!(l.candidates(), vec![missing]);
        assert_eq!(l.find(), Err(Unavailable::NotFound));
    }

    #[test]
    fn the_bundle_location_wins_over_the_development_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = if cfg!(target_os = "macos") {
            tmp.path().join("Atlas.app/Contents/MacOS/atlas")
        } else {
            tmp.path().join("app/atlas")
        };
        let bundled = if cfg!(target_os = "macos") {
            tmp.path()
                .join("Atlas.app/Contents/MacOS/../Frameworks")
                .join(LIBRARY_FILE)
        } else if cfg!(target_os = "windows") {
            tmp.path().join("app").join(LIBRARY_FILE)
        } else {
            tmp.path().join("app/../lib/Atlas").join(LIBRARY_FILE)
        };
        let vendor = tmp.path().join("vendor").join(LIBRARY_FILE);
        touch(&vendor);
        let l = Locator {
            exe,
            dev_vendor: Some(vendor.clone()),
            ..Locator::default()
        };
        assert_eq!(l.find(), Ok(vendor), "only the development copy exists");
        touch(&bundled);
        assert_eq!(l.find(), Ok(bundled));
    }

    #[test]
    fn release_builds_have_no_development_fallback() {
        let l = Locator {
            exe: PathBuf::from("/nowhere/atlas"),
            ..Locator::default()
        };
        assert_eq!(l.find(), Err(Unavailable::NotFound));
    }
}
