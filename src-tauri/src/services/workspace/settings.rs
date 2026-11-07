//! `config/settings.json`: portable, non-secret preferences.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::layout::{SETTINGS, TMP};
use crate::domain::dto::Settings;
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::fsutil::atomic_write;

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsFile {
    #[serde(default)]
    schema_version: u32,
    #[serde(flatten)]
    settings: Settings,
}

/// Missing or unreadable settings fall back to defaults; preferences are
/// never a reason to refuse opening a library.
pub fn read(root: &Path) -> Settings {
    fs::read(root.join(SETTINGS))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<SettingsFile>(&bytes).ok())
        .map(|file| file.settings)
        .unwrap_or_default()
}

pub fn exists(root: &Path) -> bool {
    root.join(SETTINGS).is_file()
}

pub fn write(root: &Path, settings: &Settings) -> AtlasResult<()> {
    let file = SettingsFile {
        schema_version: SCHEMA_VERSION,
        settings: settings.clone(),
    };
    let mut bytes = serde_json::to_vec_pretty(&file)
        .map_err(|_| AtlasError::Internal("settings could not be encoded"))?;
    bytes.push(b'\n');
    atomic_write(&root.join(SETTINGS), &root.join(TMP), &bytes)
        .map_err(AtlasError::io("save settings"))
}
