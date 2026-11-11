use tauri::State;

use crate::domain::dto::KnowledgeStatus;
use crate::error::AtlasResult;
use crate::state::AppState;

/// The knowledge store's state, size, counts and snapshots (Settings ›
/// Storage, Slice 3.2). A store that can't be used reports why here; it
/// never stops the library from opening.
#[tauri::command]
pub async fn get_knowledge_status(state: State<'_, AppState>) -> AtlasResult<KnowledgeStatus> {
    state
        .with_workspace(|ws| Ok(ws.require_open()?.knowledge().status()))
        .await
}
