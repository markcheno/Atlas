//! Debug builds only: hooks for the automated smoke test (`e2e/smoke.mjs`).
//!
//! WebDriver cannot drive native folder dialogs or see what the OS opens,
//! so a debug build can be told which folders the pickers "return" and to
//! record open/reveal requests instead of launching apps. Release builds
//! ignore these variables (the `cfg!(debug_assertions)` checks below), the
//! same way they ignore `ATLAS_CONFIG_DIR` and `ATLAS_SIMULATE_*`.

use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::domain::traits::SourceOpener;
use crate::error::{AtlasError, AtlasResult};
use crate::services::source_open::OsOpener;

/// The folder the data-root picker returns instead of showing a dialog.
pub const PICK_DATA_ROOT_ENV: &str = "ATLAS_E2E_PICK_DATA_ROOT";
/// Folders (an OS path list, like `PATH`) the library-folder picker returns.
pub const PICK_SOURCES_ENV: &str = "ATLAS_E2E_PICK_SOURCES";
/// A file to which open and reveal requests are appended instead of
/// launching the default app or file manager.
pub const OPENER_LOG_ENV: &str = "ATLAS_E2E_OPENER_LOG";

fn var(name: &str) -> Option<std::ffi::OsString> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os(name).filter(|v| !v.is_empty())
}

/// WebView2's default extra arguments in wry (`webview2/mod.rs`).
pub const WRY_DEFAULT_WEBVIEW2_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

/// Windows smoke tests: msedgedriver asks WebView2 for a remote-debugging
/// port through `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, but wry always
/// passes its own arguments, which take precedence, so the driver can never
/// attach ("DevToolsActivePort file doesn't exist"). In a debug build with
/// that variable set, the main window passes wry's defaults plus the
/// driver's arguments. Release builds and normal runs are unchanged.
pub fn webview2_driver_args() -> Option<String> {
    var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")
        .map(|extra| format!("{WRY_DEFAULT_WEBVIEW2_ARGS} {}", extra.to_string_lossy()))
}

/// The scripted data-root pick, if any.
pub fn scripted_data_root() -> Option<PathBuf> {
    var(PICK_DATA_ROOT_ENV).map(PathBuf::from)
}

/// The scripted library-folder pick, if any.
pub fn scripted_sources() -> Option<Vec<PathBuf>> {
    var(PICK_SOURCES_ENV).map(|v| parse_path_list(&v))
}

pub fn parse_path_list(value: &OsStr) -> Vec<PathBuf> {
    std::env::split_paths(value)
        .filter(|p| !p.as_os_str().is_empty())
        .collect()
}

/// The opener the app uses: the OS one, or a recorder in smoke tests.
pub fn opener() -> Arc<dyn SourceOpener> {
    match var(OPENER_LOG_ENV) {
        Some(log) => Arc::new(RecordingOpener {
            log: PathBuf::from(log),
        }),
        None => Arc::new(OsOpener),
    }
}

/// Appends `open <path>` or `reveal <path>` lines to a file.
pub struct RecordingOpener {
    pub log: PathBuf,
}

impl RecordingOpener {
    fn record(&self, action: &str, path: &Path) -> AtlasResult<()> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .map_err(|_| AtlasError::OpenFailed)?;
        writeln!(f, "{action} {}", path.display()).map_err(|_| AtlasError::OpenFailed)
    }
}

impl SourceOpener for RecordingOpener {
    fn open_original(&self, path: &Path) -> AtlasResult<()> {
        self.record("open", path)
    }

    fn reveal_original(&self, path: &Path) -> AtlasResult<()> {
        self.record("reveal", path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_lists_use_the_os_separator_and_skip_empties() {
        let a = std::env::temp_dir().join("a b");
        let b = std::env::temp_dir().join("café");
        let joined = std::env::join_paths([&a, &PathBuf::new(), &b]).unwrap();
        assert_eq!(parse_path_list(&joined), vec![a, b]);
    }

    #[test]
    fn the_recorder_appends_one_line_per_request() {
        let dir = tempfile::tempdir().unwrap();
        let rec = RecordingOpener {
            log: dir.path().join("opened.log"),
        };
        rec.open_original(Path::new("/books/x.pdf")).unwrap();
        rec.reveal_original(Path::new("/books/y.epub")).unwrap();
        let text = std::fs::read_to_string(&rec.log).unwrap();
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [
                format!("open {}", Path::new("/books/x.pdf").display()),
                format!("reveal {}", Path::new("/books/y.epub").display()),
            ]
        );
    }
}
