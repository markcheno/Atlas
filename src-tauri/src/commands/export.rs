//! Export and import (Slice 3.8, phase3-spec §5.8, §7.7, §9; ADR 0019).
//!
//! The renderer never sends a path: Rust shows the folder and file
//! dialogs itself, outside the workspace lock, and the work runs without
//! the lock on handles copied out under it.

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::domain::dto::{
    ExportFormat, ExportReport, ExportScope, ImportPreview, ImportReport, LinkStyle,
};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::e2e;
use crate::services::export::{self, Handles};
use crate::state::AppState;

async fn handles(state: &State<'_, AppState>) -> AtlasResult<Handles> {
    state
        .with_workspace(|ws| Handles::of(ws.require_open()?))
        .await
}

async fn blocking<R: Send + 'static>(
    f: impl FnOnce() -> AtlasResult<R> + Send + 'static,
) -> AtlasResult<R> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| AtlasError::Internal("a background task failed"))?
}

/// Exports `scope` as `format` into the `Atlas` folder of a folder chosen
/// in the native dialog. `None` when the dialog was cancelled.
#[tauri::command]
pub async fn export_knowledge(
    app: AppHandle,
    state: State<'_, AppState>,
    scope: ExportScope,
    format: ExportFormat,
    link_style: Option<LinkStyle>,
) -> AtlasResult<Option<ExportReport>> {
    let h = handles(&state).await?;
    let chosen = match e2e::scripted_export_dir() {
        Some(path) => Some(path),
        None => {
            let start = h.start_folder();
            let picked = blocking(move || {
                Ok(app
                    .dialog()
                    .file()
                    .set_title("Export to a folder")
                    .set_directory(start)
                    .set_can_create_directories(true)
                    .blocking_pick_folder())
            })
            .await?;
            match picked {
                Some(p) => Some(p.into_path().map_err(|_| AtlasError::InvalidPath)?),
                None => None,
            }
        }
    };
    let Some(chosen) = chosen else {
        return Ok(None);
    };
    let style = link_style.unwrap_or_default();
    blocking(move || export::export(&h, &scope, format, style, &chosen))
        .await
        .map(Some)
}

/// Reads a JSON export chosen in the native dialog and says what importing
/// it would do. Nothing is written. `None` when the dialog was cancelled.
#[tauri::command]
pub async fn preview_import(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AtlasResult<Option<ImportPreview>> {
    let h = handles(&state).await?;
    let path = match e2e::scripted_import_file() {
        Some(path) => Some(path),
        None => {
            let start = h.start_folder();
            let picked = blocking(move || {
                Ok(app
                    .dialog()
                    .file()
                    .set_title("Import from a file")
                    .set_directory(start)
                    .add_filter("Atlas export", &["json"])
                    .blocking_pick_file())
            })
            .await?;
            match picked {
                Some(p) => Some(p.into_path().map_err(|_| AtlasError::InvalidPath)?),
                None => None,
            }
        }
    };
    let Some(path) = path else {
        return Ok(None);
    };
    blocking(move || export::preview(&h, &path)).await.map(Some)
}

/// Imports the file the dry run named `token` read.
#[tauri::command]
pub async fn apply_import(state: State<'_, AppState>, token: String) -> AtlasResult<ImportReport> {
    let h = handles(&state).await?;
    blocking(move || export::apply(&h, &token)).await
}
