//! `config/settings.json`: portable, non-secret preferences.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::layout::{SETTINGS, TMP};
use crate::domain::dto::Settings;
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::fsutil::atomic_write;

const SCHEMA_VERSION: u32 = 1;

/// What Rust keeps beside the settings and never sends to the renderer
/// (Slice 3.8): the renderer supplies no paths.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ExportState {
    /// The folder the last export went to, where the next dialog starts.
    last_folder: Option<String>,
}

impl ExportState {
    fn is_empty(&self) -> bool {
        self.last_folder.is_none()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsFile {
    #[serde(default)]
    schema_version: u32,
    #[serde(default, skip_serializing_if = "ExportState::is_empty")]
    export: ExportState,
    #[serde(flatten)]
    settings: Settings,
}

fn read_file(root: &Path) -> Option<SettingsFile> {
    fs::read(root.join(SETTINGS))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<SettingsFile>(&bytes).ok())
}

/// Missing or unreadable settings fall back to defaults; preferences are
/// never a reason to refuse opening a library.
pub fn read(root: &Path) -> Settings {
    read_file(root)
        .map(|file| file.settings)
        .unwrap_or_default()
}

/// The folder the last export went to, if it is still a folder.
pub fn last_export_folder(root: &Path) -> Option<PathBuf> {
    read_file(root)
        .and_then(|f| f.export.last_folder)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// Remembers the folder an export went to. A path that isn't Unicode
/// isn't kept: the next dialog starts in `exports` instead.
pub fn set_last_export_folder(root: &Path, folder: &Path) -> AtlasResult<()> {
    let Some(text) = folder.to_str() else {
        return Ok(());
    };
    let mut file = read_file(root).unwrap_or(SettingsFile {
        schema_version: SCHEMA_VERSION,
        export: ExportState::default(),
        settings: Settings::default(),
    });
    file.export.last_folder = Some(text.to_owned());
    save(root, &file)
}

pub fn exists(root: &Path) -> bool {
    root.join(SETTINGS).is_file()
}

/// Saves the renderer's settings, keeping what Rust keeps beside them.
pub fn write(root: &Path, settings: &Settings) -> AtlasResult<()> {
    let file = SettingsFile {
        schema_version: SCHEMA_VERSION,
        export: read_file(root).map(|f| f.export).unwrap_or_default(),
        settings: settings.clone(),
    };
    save(root, &file)
}

fn save(root: &Path, file: &SettingsFile) -> AtlasResult<()> {
    let mut bytes = serde_json::to_vec_pretty(file)
        .map_err(|_| AtlasError::Internal("settings could not be encoded"))?;
    bytes.push(b'\n');
    atomic_write(&root.join(SETTINGS), &root.join(TMP), &bytes)
        .map_err(AtlasError::io("save settings"))
}

#[cfg(test)]
mod export_tests {
    use super::*;

    #[test]
    fn the_last_export_folder_survives_the_renderers_settings() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("config")).unwrap();
        fs::create_dir_all(root.path().join(TMP)).unwrap();
        write(root.path(), &Settings::default()).unwrap();
        assert_eq!(last_export_folder(root.path()), None);
        let out = tempfile::tempdir().unwrap();
        set_last_export_folder(root.path(), out.path()).unwrap();
        assert_eq!(last_export_folder(root.path()).as_deref(), Some(out.path()));
        // The renderer sends Settings without it: it stays.
        write(root.path(), &Settings::default()).unwrap();
        assert_eq!(last_export_folder(root.path()).as_deref(), Some(out.path()));
        let text = fs::read_to_string(root.path().join(SETTINGS)).unwrap();
        assert!(text.contains("lastFolder"));
        // A folder gone since: the dialog starts elsewhere.
        drop(out);
        assert_eq!(last_export_folder(root.path()), None);
    }
}
