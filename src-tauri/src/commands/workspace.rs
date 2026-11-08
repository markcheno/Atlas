use std::path::PathBuf;

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::domain::dto::{DataRootCandidate, DataRootStatus};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::e2e;
use crate::state::AppState;

#[tauri::command]
pub async fn get_data_root_status(state: State<'_, AppState>) -> AtlasResult<DataRootStatus> {
    state.with_workspace(|ws| Ok(ws.status())).await
}

/// The suggested location for a new data root on this computer.
#[tauri::command]
pub async fn get_default_data_root(state: State<'_, AppState>) -> AtlasResult<String> {
    state.with_workspace(|ws| Ok(ws.default_root())).await
}

/// Shows the native folder picker. Runs on the Rust side so the renderer
/// needs no dialog or filesystem permission. `None` means cancelled.
#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> AtlasResult<Option<String>> {
    if let Some(path) = e2e::scripted_data_root() {
        return path
            .to_str()
            .map(|s| Some(s.to_owned()))
            .ok_or(AtlasError::InvalidPath);
    }
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_can_create_directories(true)
            .blocking_pick_folder()
    })
    .await
    .map_err(|_| AtlasError::Internal("the folder picker failed"))?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|_| AtlasError::InvalidPath)?;
    path.to_str()
        .map(|s| Some(s.to_owned()))
        .ok_or(AtlasError::InvalidPath)
}

/// Checks a prospective data-root location without creating it.
#[tauri::command]
pub async fn inspect_data_root(
    state: State<'_, AppState>,
    path: String,
) -> AtlasResult<DataRootCandidate> {
    state
        .with_workspace(move |ws| ws.inspect(&PathBuf::from(path)))
        .await
}

/// Creates a new Atlas Data Root at `path`, or reopens a compatible one.
#[tauri::command]
pub async fn choose_data_root(
    state: State<'_, AppState>,
    path: String,
) -> AtlasResult<DataRootStatus> {
    state
        .with_workspace(move |ws| ws.choose(&PathBuf::from(path)))
        .await
}

/// Opens an existing Atlas Data Root at `path` without creating anything.
#[tauri::command]
pub async fn open_data_root(
    state: State<'_, AppState>,
    path: String,
) -> AtlasResult<DataRootStatus> {
    state
        .with_workspace(move |ws| ws.open(&PathBuf::from(path)))
        .await
}
