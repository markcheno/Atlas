use tauri::State;

use crate::domain::dto::{ExtractionOverview, JobKind};
use crate::error::{AtlasError, AtlasResult};
use crate::services::extraction::db;
use crate::services::jobs;
use crate::state::AppState;

/// "Read text now": schedules reading the text of one library folder's
/// files, or returns the job already scheduled. Returns the job ID.
#[tauri::command]
pub async fn start_extraction(
    state: State<'_, AppState>,
    source_root_id: String,
) -> AtlasResult<String> {
    state
        .with_workspace(move |ws| {
            jobs::control::enqueue(ws.require_open()?, &source_root_id, JobKind::Extract)
        })
        .await
}

/// Text reading status; also pushed as the `extraction-status` event.
#[tauri::command]
pub async fn get_extraction_status(state: State<'_, AppState>) -> AtlasResult<ExtractionOverview> {
    state
        .with_workspace(|ws| {
            ws.require_open()?
                .reader()
                .read(|c| db::overview(c, None))?
                .map_err(AtlasError::db("read text status"))
        })
        .await
}
