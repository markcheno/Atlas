use tauri::State;

use crate::domain::dto::{CleanCachesResult, StorageUsage};
use crate::error::{AtlasError, AtlasResult};
use crate::services::storage;
use crate::state::AppState;

/// What the library's data takes on disk, with the indexing estimate.
/// Measured on a connection of its own, outside the workspace lock.
#[tauri::command]
pub async fn get_storage_usage(state: State<'_, AppState>) -> AtlasResult<StorageUsage> {
    let (root, volumes) = state
        .with_workspace(|ws| Ok((ws.require_open()?.path().to_path_buf(), ws.volumes())))
        .await?;
    tauri::async_runtime::spawn_blocking(move || storage::usage(&root, volumes.as_ref()))
        .await
        .map_err(|_| AtlasError::Internal("a background task failed"))?
}

/// Empties the rebuildable caches (`cache/`, `tmp/`) and reports what that
/// freed. Never touches the catalog, the text or the settings.
#[tauri::command]
pub async fn clean_caches(state: State<'_, AppState>) -> AtlasResult<CleanCachesResult> {
    let (freed, root, volumes) = state
        .with_workspace(|ws| {
            let root = ws.require_open()?.path().to_path_buf();
            let freed = storage::clean_caches(&root)?;
            Ok((freed, root, ws.volumes()))
        })
        .await?;
    let usage =
        tauri::async_runtime::spawn_blocking(move || storage::usage(&root, volumes.as_ref()))
            .await
            .map_err(|_| AtlasError::Internal("a background task failed"))??;
    Ok(CleanCachesResult {
        freed_bytes: freed,
        usage,
    })
}
