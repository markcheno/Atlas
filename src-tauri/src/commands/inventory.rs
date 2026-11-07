use crate::domain::dto::InventoryStatus;
use crate::error::{AtlasError, AtlasResult};

/// Starts (or resumes) inventory of one source root; returns the job ID.
#[tauri::command]
pub async fn start_inventory(source_root_id: String) -> AtlasResult<String> {
    let _ = source_root_id;
    Err(AtlasError::not_implemented("start_inventory", "0.5"))
}

#[tauri::command]
pub async fn pause_job(job_id: String) -> AtlasResult<()> {
    let _ = job_id;
    Err(AtlasError::not_implemented("pause_job", "0.5"))
}

#[tauri::command]
pub async fn resume_job(job_id: String) -> AtlasResult<()> {
    let _ = job_id;
    Err(AtlasError::not_implemented("resume_job", "0.5"))
}

/// Stops the job, keeping already cataloged entries.
#[tauri::command]
pub async fn cancel_job(job_id: String) -> AtlasResult<()> {
    let _ = job_id;
    Err(AtlasError::not_implemented("cancel_job", "0.5"))
}

#[tauri::command]
pub async fn get_inventory_status() -> AtlasResult<InventoryStatus> {
    Err(AtlasError::not_implemented("get_inventory_status", "0.5"))
}
