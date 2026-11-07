use crate::domain::dto::{AppStatus, BuildProfile, DeviceSnapshot, FeatureStatus, OsKind};
use crate::error::{AtlasError, AtlasResult};

/// The Phase 0 slice this build implements.
pub const CURRENT_SLICE: &str = "0.1";

const PLANNED_FEATURES: &[(&str, &str, &str)] = &[
    ("data_root", "Atlas data location", "0.2"),
    ("source_folders", "Library folders", "0.3"),
    ("device_check", "Device check", "0.4"),
    ("inventory", "Background cataloging", "0.5"),
    ("library_browse", "Browse and open files", "0.6"),
    ("provider_test", "AI endpoint connection test", "0.7"),
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
        features: PLANNED_FEATURES
            .iter()
            .map(|(id, label, slice)| FeatureStatus {
                id: (*id).to_owned(),
                label: (*label).to_owned(),
                implemented: false,
                planned_slice: Some((*slice).to_owned()),
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
