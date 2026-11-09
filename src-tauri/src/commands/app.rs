use tauri::State;

use crate::domain::dto::{
    AppStatus, BuildProfile, DeviceCheck, DeviceSnapshot, FeatureStatus, OsKind,
};
use crate::error::AtlasResult;
use crate::state::AppState;

/// The slice this build implements, "<phase>.<slice>" (Phase 1, Slice 1 is
/// "1.1"). The UI derives the phase from it.
pub const CURRENT_SLICE: &str = "1.7";

/// (id, label, delivering slice, implemented in this build)
const FEATURES: &[(&str, &str, &str, bool)] = &[
    ("data_root", "Atlas data location", "0.2", true),
    ("source_folders", "Library folders", "0.3", true),
    ("device_check", "Device check", "0.4", true),
    ("inventory", "Background cataloging", "0.5", true),
    ("library_browse", "Browse and open files", "0.6", true),
    ("provider_test", "AI endpoint connection test", "0.7", true),
    ("extraction", "Read text from PDFs and EPUBs", "1.3", true),
    (
        "full_text_search",
        "Search the text of your books and papers",
        "1.4",
        true,
    ),
    (
        "search_screen",
        "Search screen and document details",
        "1.5",
        true,
    ),
    (
        "metadata_quality",
        "Titles from the first page or the file name",
        "1.6",
        true,
    ),
    ("duplicates", "Duplicate files by content", "1.6", true),
    ("storage_usage", "Storage use and Clean caches", "1.6", true),
];

/// Builds the status from facts about this process only.
pub fn app_status() -> AppStatus {
    AppStatus {
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        os: OsKind::current(),
        arch: std::env::consts::ARCH.to_owned(),
        build_profile: if cfg!(debug_assertions) {
            BuildProfile::Debug
        } else {
            BuildProfile::Release
        },
        slice: CURRENT_SLICE.to_owned(),
        features: FEATURES
            .iter()
            .map(|(id, label, slice, implemented)| FeatureStatus {
                id: (*id).to_owned(),
                label: (*label).to_owned(),
                implemented: *implemented,
                planned_slice: (!implemented).then(|| (*slice).to_owned()),
            })
            .collect(),
    }
}

#[tauri::command]
pub async fn get_app_status() -> AtlasResult<AppStatus> {
    Ok(app_status())
}

/// A fresh device reading. Data-drive fields are null when no root is open.
#[tauri::command]
pub async fn get_device_snapshot(state: State<'_, AppState>) -> AtlasResult<DeviceSnapshot> {
    state.device_snapshot().await
}

/// Reading, plain-language assessment, and what background work may do.
#[tauri::command]
pub async fn get_device_check(state: State<'_, AppState>) -> AtlasResult<DeviceCheck> {
    state.device_check().await
}
