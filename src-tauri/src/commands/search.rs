use tauri::State;

use crate::domain::dto::{SearchPage, SearchQuery};
use crate::error::AtlasResult;
use crate::services::search;
use crate::state::AppState;

/// Full-text search over the text Atlas has read, grouped by document,
/// best match first (phase1-spec §7.4).
#[tauri::command]
pub async fn search_documents(
    state: State<'_, AppState>,
    query: SearchQuery,
) -> AtlasResult<SearchPage> {
    state
        .with_workspace(move |ws| {
            let root = ws.require_open()?;
            search::run(root.reader(), root.root_status(), &query)
        })
        .await
}
