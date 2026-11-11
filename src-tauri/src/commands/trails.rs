//! Research trails (Slice 3.3, phase3-spec §7.1, §9; ADR 0017).
//!
//! The renderer sends ids, never a step's content: what a step holds is
//! taken in Rust from the catalog and the search or answer that made it.
//! Each command takes the library's recorder and lets the workspace go
//! before touching the store.

use std::sync::Arc;

use tauri::State;

use crate::domain::dto::{
    OpenAction, OpenPlace, OpenVia, RecordingStatus, TrailDetail, TrailPage, TrailQuery,
    TrailSummary,
};
use crate::error::{AtlasError, AtlasResult};
use crate::services::knowledge::Recorder;
use crate::state::AppState;

/// Runs `f` with the open library's recorder, off the workspace lock.
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

/// A source the user opened: the details panel, Open, Show in the file
/// manager. Rust builds its anchor from the catalog; from the Library it
/// is not a step. Never fails the open: a problem shows in the indicator.
#[tauri::command]
pub async fn record_open(
    state: State<'_, AppState>,
    file_id: String,
    via: OpenVia,
    action: OpenAction,
    place: Option<OpenPlace>,
) -> AtlasResult<()> {
    with_recorder(&state, move |r| {
        r.open(&file_id, &via, action, place.unwrap_or_default());
        Ok(())
    })
    .await
}

/// The status bar's indicator: recording, paused or unavailable, the
/// current trail, Home's one-time note, and whether a step failed.
#[tauri::command]
pub async fn get_recording_status(state: State<'_, AppState>) -> AtlasResult<RecordingStatus> {
    with_recorder(&state, |r| Ok(r.status())).await
}

/// Dismisses Home's note about trails.
#[tauri::command]
pub async fn dismiss_trail_note(state: State<'_, AppState>) -> AtlasResult<RecordingStatus> {
    with_recorder(&state, Recorder::dismiss_note).await
}

/// A page of trails, newest activity first, filtered by titles, questions
/// and searches.
#[tauri::command]
pub async fn list_trails(state: State<'_, AppState>, query: TrailQuery) -> AtlasResult<TrailPage> {
    with_recorder(&state, move |r| r.list(&query)).await
}

/// A trail's steps, oldest first, with every anchor resolved against the
/// catalog now. Nothing is written.
#[tauri::command]
pub async fn get_trail(state: State<'_, AppState>, trail_id: String) -> AtlasResult<TrailDetail> {
    with_recorder(&state, move |r| r.get(&trail_id)).await
}

/// "New trail": the next step starts one.
#[tauri::command]
pub async fn start_trail(state: State<'_, AppState>) -> AtlasResult<RecordingStatus> {
    with_recorder(&state, Recorder::start).await
}

/// "Continue this trail": new steps join it.
#[tauri::command]
pub async fn continue_trail(
    state: State<'_, AppState>,
    trail_id: String,
) -> AtlasResult<TrailSummary> {
    with_recorder(&state, move |r| r.continue_trail(&trail_id)).await
}

/// Gives a trail a title (empty takes it away), up to 200 characters.
#[tauri::command]
pub async fn rename_trail(
    state: State<'_, AppState>,
    trail_id: String,
    title: String,
) -> AtlasResult<TrailSummary> {
    with_recorder(&state, move |r| r.rename(&trail_id, &title)).await
}

/// A pinned trail never expires.
#[tauri::command]
pub async fn pin_trail(
    state: State<'_, AppState>,
    trail_id: String,
    pinned: bool,
) -> AtlasResult<TrailSummary> {
    with_recorder(&state, move |r| r.pin(&trail_id, pinned)).await
}

/// Deletes a trail and its steps. Findings saved from it stay whole.
#[tauri::command]
pub async fn delete_trail(state: State<'_, AppState>, trail_id: String) -> AtlasResult<()> {
    with_recorder(&state, move |r| r.delete(&trail_id)).await
}

/// Deletes one step; a trail left without steps goes too.
#[tauri::command]
pub async fn delete_trail_step(state: State<'_, AppState>, step_id: String) -> AtlasResult<()> {
    with_recorder(&state, move |r| r.delete_step(&step_id)).await
}

/// A note of up to 4,000 characters: on a step, or as a step of its own.
/// Written while recording is paused too.
#[tauri::command]
pub async fn add_trail_note(
    state: State<'_, AppState>,
    trail_id: String,
    step_id: Option<String>,
    text: String,
) -> AtlasResult<()> {
    with_recorder(&state, move |r| {
        r.note(&trail_id, step_id.as_deref(), &text)
    })
    .await
}

/// Deletes every trail; bookmarks and findings are kept. The number
/// deleted.
#[tauri::command]
pub async fn delete_all_trails(state: State<'_, AppState>) -> AtlasResult<u32> {
    with_recorder(&state, Recorder::delete_all).await
}
