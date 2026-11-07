use crate::domain::dto::{AppStatus, BuildProfile, DeviceSnapshot, FeatureStatus, OsKind};
use crate::error::{AtlasError, AtlasResult};

/// The Phase 0 slice this build implements.
pub const CURRENT_SLICE: &str = "0.3";

/// (id, label, delivering slice, implemented in this build)
const FEATURES: &[(&str, &str, &str, bool)] = &[
    ("data_root", "Atlas data location", "0.2", true),
    ("source_folders", "Library folders", "0.3", true),
    ("device_check", "Device check", "0.4", false),
    ("inventory", "Background cataloging", "0.5", false),
    ("library_browse", "Browse and open files", "0.6", false),
    ("provider_test", "AI endpoint connection test", "0.7", false),
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

#[tauri::command]
pub async fn get_device_snapshot() -> AtlasResult<DeviceSnapshot> {
    Err(AtlasError::not_implemented("get_device_snapshot", "0.4"))
}
