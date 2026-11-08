use std::path::PathBuf;

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::domain::dto::{AddSourceRootsResult, SourceRoot};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::e2e;
use crate::services::inventory::control;
use crate::services::sources;
use crate::state::AppState;

/// Library folders with live availability.
#[tauri::command]
pub async fn list_source_roots(state: State<'_, AppState>) -> AtlasResult<Vec<SourceRoot>> {
    state
        .with_workspace(|ws| sources::list(ws.require_open()?))
        .await
}

/// Adds one folder by path and starts cataloging it. Paths over IPC are
/// UTF-8; folders with other names are added through
/// `pick_and_add_source_roots`.
#[tauri::command]
pub async fn add_source_root(state: State<'_, AppState>, path: String) -> AtlasResult<SourceRoot> {
    state
        .with_workspace(move |ws| {
            let root = ws.require_open()?;
            let added = sources::add(root, &PathBuf::from(path))?;
            control::enqueue(root, &added.id)?;
            Ok(added)
        })
        .await
}

/// Removes a folder from Atlas. Never deletes or changes files on disk.
#[tauri::command]
pub async fn remove_source_root(state: State<'_, AppState>, id: String) -> AtlasResult<()> {
    state
        .with_workspace(move |ws| sources::remove(ws.require_open()?, &id))
        .await
}

/// Shows the native multi-folder picker, adds every chosen folder, and
/// starts cataloging each one.
/// Native paths stay on the Rust side, so any folder name works.
#[tauri::command]
pub async fn pick_and_add_source_roots(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AtlasResult<AddSourceRootsResult> {
    let paths: Vec<PathBuf> = match e2e::scripted_sources() {
        Some(paths) => paths,
        None => {
            let picked = tauri::async_runtime::spawn_blocking(move || {
                app.dialog()
                    .file()
                    .set_title("Add library folders")
                    .blocking_pick_folders()
            })
            .await
            .map_err(|_| AtlasError::Internal("the folder picker failed"))?;
            picked
                .unwrap_or_default()
                .into_iter()
                .filter_map(|p| p.into_path().ok())
                .collect()
        }
    };
    if paths.is_empty() {
        return Ok(AddSourceRootsResult::default());
    }
    state
        .with_workspace(move |ws| {
            let root = ws.require_open()?;
            let result = sources::add_many(root, &paths);
            for added in &result.added {
                control::enqueue(root, &added.id)?;
            }
            Ok(result)
        })
        .await
}
