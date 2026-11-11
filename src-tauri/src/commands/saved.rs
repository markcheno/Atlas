//! Bookmarks and saved findings (Slice 3.5, phase3-spec §5.3, §7.3, §9).
//!
//! The renderer names what to keep (a file and a place, an answer, a
//! trail step); Rust builds it from the catalog and from what it showed.

use std::sync::Arc;

use tauri::State;

use crate::domain::dto::{
    Bookmark, Finding, FindingSource, SaveTarget, SavedItem, SavedKind, SavedPage, SavedQuery,
};
use crate::error::{AtlasError, AtlasResult};
use crate::services::knowledge::{Recorder, saved};
use crate::state::AppState;

async fn with_recorder<R, F>(state: &State<'_, AppState>, f: F) -> AtlasResult<R>
where
    R: Send + 'static,
    F: FnOnce(&Recorder) -> AtlasResult<R> + Send + 'static,
{
    let recorder = state
        .with_workspace(|ws| Ok(Arc::clone(ws.require_open()?.recorder())))
        .await?;
    tauri::async_runtime::spawn_blocking(move || f(&recorder))
        .await
        .map_err(|_| AtlasError::Internal("a background task failed"))?
}

/// Bookmarks a place; the same place again updates the bookmark there.
#[tauri::command]
pub async fn save_bookmark(
    state: State<'_, AppState>,
    target: SaveTarget,
) -> AtlasResult<Bookmark> {
    with_recorder(&state, move |r| saved::save_bookmark(r, &target)).await
}

/// Saves an answer (with its question and cited sources) or a passage.
#[tauri::command]
pub async fn save_finding(
    state: State<'_, AppState>,
    source: FindingSource,
) -> AtlasResult<Finding> {
    with_recorder(&state, move |r| saved::save_finding(r, &source)).await
}

/// A bookmark's or finding's note, up to 4,000 characters; empty clears it.
#[tauri::command]
pub async fn update_saved_note(
    state: State<'_, AppState>,
    kind: SavedKind,
    id: String,
    note: String,
) -> AtlasResult<SavedItem> {
    with_recorder(&state, move |r| saved::set_note(r, kind, &id, &note)).await
}

/// Deletes a bookmark or finding, and takes it out of every collection.
#[tauri::command]
pub async fn delete_saved(
    state: State<'_, AppState>,
    kind: SavedKind,
    id: String,
) -> AtlasResult<()> {
    with_recorder(&state, move |r| saved::delete(r, kind, &id)).await
}

/// Bookmarks and findings, newest first, each source resolved now.
#[tauri::command]
pub async fn list_saved(state: State<'_, AppState>, query: SavedQuery) -> AtlasResult<SavedPage> {
    with_recorder(&state, move |r| saved::list(r, &query)).await
}
