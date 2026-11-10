use tauri::State;

use crate::domain::dto::EmbeddingOverview;
use crate::error::{AtlasError, AtlasResult};
use crate::services::embedding::{status, triggers};
use crate::state::AppState;

/// "Index meaning now": schedules indexing one library folder's meaning,
/// retries a job that failed for now, or returns the job already
/// scheduled. Returns the job ID.
#[tauri::command]
pub async fn start_embedding(
    state: State<'_, AppState>,
    source_root_id: String,
) -> AtlasResult<String> {
    state
        .with_workspace(move |ws| triggers::start(ws.require_open()?, &source_root_id))
        .await
}

/// Meaning-index status; also pushed as the `embedding-status` event.
#[tauri::command]
pub async fn get_embedding_status(state: State<'_, AppState>) -> AtlasResult<EmbeddingOverview> {
    state
        .with_workspace(|ws| {
            let root = ws.require_open()?;
            let allow =
                crate::services::providers::privacy::remote_embedding(&root.settings().privacy);
            root.reader()
                .read(move |c| status::overview(c, allow))?
                .map_err(AtlasError::db("read the meaning index status"))
        })
        .await
}
