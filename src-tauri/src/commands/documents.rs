use tauri::State;

use crate::domain::dto::DocumentDetail;
use crate::error::AtlasResult;
use crate::services::documents;
use crate::state::AppState;

/// What the detail panel shows for one file: its document's title, authors,
/// language and pages, its text status, and the other copies of its content.
#[tauri::command]
pub async fn get_document_detail(
    state: State<'_, AppState>,
    file_id: String,
) -> AtlasResult<DocumentDetail> {
    state
        .with_workspace(move |ws| {
            let root = ws.require_open()?;
            documents::detail(root.reader(), root.root_status(), &file_id)
        })
        .await
}
