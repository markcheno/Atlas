use tauri::State;

use crate::domain::dto::{InventoryOverview, ScanProblem, ScanRecord};
use crate::error::AtlasResult;
use crate::services::inventory::control;
use crate::services::jobs;
use crate::state::AppState;

/// Starts (or resumes) cataloging one library folder; returns the job ID.
#[tauri::command]
pub async fn start_inventory(
    state: State<'_, AppState>,
    source_root_id: String,
) -> AtlasResult<String> {
    state
        .with_workspace(move |ws| control::rescan(ws.require_open()?, &source_root_id))
        .await
}

/// Pauses a job of any kind until the user resumes it, across restarts.
#[tauri::command]
pub async fn pause_job(state: State<'_, AppState>, job_id: String) -> AtlasResult<()> {
    state
        .with_workspace(move |ws| jobs::control::pause(ws.require_open()?, &job_id))
        .await
}

#[tauri::command]
pub async fn resume_job(state: State<'_, AppState>, job_id: String) -> AtlasResult<()> {
    state
        .with_workspace(move |ws| jobs::control::resume(ws.require_open()?, &job_id))
        .await
}

/// Stops the job, keeping already cataloged entries.
#[tauri::command]
pub async fn cancel_job(state: State<'_, AppState>, job_id: String) -> AtlasResult<()> {
    state
        .with_workspace(move |ws| jobs::control::cancel(ws.require_open()?, &job_id))
        .await
}

#[tauri::command]
pub async fn get_inventory_status(state: State<'_, AppState>) -> AtlasResult<InventoryOverview> {
    state
        .with_workspace(|ws| control::overview(ws.require_open()?))
        .await
}

/// A folder's recent checks, newest first (Slice 4.5).
#[tauri::command]
pub async fn list_scans(
    state: State<'_, AppState>,
    source_root_id: String,
    limit: Option<u32>,
) -> AtlasResult<Vec<ScanRecord>> {
    state
        .with_workspace(move |ws| {
            control::scans(ws.require_open()?, &source_root_id, limit.unwrap_or(10))
        })
        .await
}

/// The folders a check couldn't read, and why (Slice 4.5).
#[tauri::command]
pub async fn get_scan_problems(
    state: State<'_, AppState>,
    job_id: String,
) -> AtlasResult<Vec<ScanProblem>> {
    state
        .with_workspace(move |ws| control::problems(ws.require_open()?, &job_id))
        .await
}

/// Check all folders now (Slice 4.5): returns how many have a check
/// scheduled.
#[tauri::command]
pub async fn check_all_folders(state: State<'_, AppState>) -> AtlasResult<u32> {
    state
        .with_workspace(|ws| control::check_all(ws.require_open()?))
        .await
}
