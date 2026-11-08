use tauri::State;

use crate::domain::dto::{
    AppStatus, BuildProfile, DeviceCheck, DeviceSnapshot, FeatureStatus, OsKind,
};
use crate::error::AtlasResult;
use crate::state::AppState;

/// The Phase 0 slice this build implements.
pub const CURRENT_SLICE: &str = "0.8";

/// (id, label, delivering slice, implemented in this build)
const FEATURES: &[(&str, &str, &str, bool)] = &[
    ("data_root", "Atlas data location", "0.2", true),
    ("source_folders", "Library folders", "0.3", true),
    ("device_check", "Device check", "0.4", true),
    ("inventory", "Background cataloging", "0.5", true),
    ("library_browse", "Browse and open files", "0.6", true),
    ("provider_test", "AI endpoint connection test", "0.7", true),
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
