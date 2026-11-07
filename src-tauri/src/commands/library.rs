use tauri::State;

use crate::domain::dto::{DocumentPage, DocumentQuery};
use crate::error::AtlasResult;
use crate::services::{library, source_open};
use crate::state::AppState;

/// Paginated filename/path listing of cataloged files.
#[tauri::command]
pub async fn list_documents(
    state: State<'_, AppState>,
    query: DocumentQuery,
) -> AtlasResult<DocumentPage> {
    state
        .with_workspace(move |ws| {
            let root = ws.require_open()?;
            library::list(root.reader(), root.root_status(), &query)
        })
        .await
}

/// Opens a cataloged file in the OS default application. A file stored only
/// in the cloud is opened (and so downloaded) only with `allow_download`.
#[tauri::command]
pub async fn open_original(
    state: State<'_, AppState>,
    file_id: String,
    allow_download: Option<bool>,
) -> AtlasResult<()> {
    let opener = state.opener.clone();
    state
        .with_workspace(move |ws| {
            source_open::open(
                ws.require_open()?,
                opener.as_ref(),
                &file_id,
                allow_download.unwrap_or(false),
            )
        })
        .await
}

/// Reveals a cataloged file in the OS file manager. Never downloads it.
#[tauri::command]
pub async fn reveal_original(state: State<'_, AppState>, file_id: String) -> AtlasResult<()> {
    let opener = state.opener.clone();
    state
        .with_workspace(move |ws| {
            source_open::reveal(ws.require_open()?, opener.as_ref(), &file_id)
        })
        .await
}
