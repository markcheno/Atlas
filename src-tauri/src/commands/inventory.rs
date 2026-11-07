use tauri::State;

use crate::domain::dto::InventoryOverview;
use crate::error::AtlasResult;
use crate::services::inventory::control;
use crate::state::AppState;

/// Starts (or resumes) cataloging one library folder; returns the job ID.
#[tauri::command]
pub async fn start_inventory(
    state: State<'_, AppState>,
    source_root_id: String,
) -> AtlasResult<String> {
    state
        .with_workspace(move |ws| control::enqueue(ws.require_open()?, &source_root_id))
        .await
}

/// Pauses a job until the user resumes it, across restarts.
#[tauri::command]
pub async fn pause_job(state: State<'_, AppState>, job_id: String) -> AtlasResult<()> {
    state
        .with_workspace(move |ws| control::pause(ws.require_open()?, &job_id))
        .await
}

#[tauri::command]
pub async fn resume_job(state: State<'_, AppState>, job_id: String) -> AtlasResult<()> {
    state
        .with_workspace(move |ws| control::resume(ws.require_open()?, &job_id))
        .await
}

/// Stops the job, keeping already cataloged entries.
#[tauri::command]
pub async fn cancel_job(state: State<'_, AppState>, job_id: String) -> AtlasResult<()> {
    state
        .with_workspace(move |ws| control::cancel(ws.require_open()?, &job_id))
        .await
}

#[tauri::command]
pub async fn get_inventory_status(state: State<'_, AppState>) -> AtlasResult<InventoryOverview> {
    state
        .with_workspace(|ws| control::overview(ws.require_open()?))
        .await
}
