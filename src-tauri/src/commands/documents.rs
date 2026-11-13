use tauri::State;

use crate::domain::dto::{DocumentDetail, DuplicatePage, DuplicateQuery, RelatedPage};
use crate::error::{AtlasError, AtlasResult};
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

/// Whether a file and the other format beside it, with the same name, are
/// one book (Slice 4.5.1): the owner's choice, kept over Atlas's own check.
/// Returns the file's details as they now stand.
#[tauri::command]
pub async fn set_same_book(
    state: State<'_, AppState>,
    file_id: String,
    same: bool,
) -> AtlasResult<DocumentDetail> {
    state
        .with_workspace(move |ws| {
            let root = ws.require_open()?;
            crate::services::formats::set_same_book(root, &file_id, same)?;
            documents::detail(root.reader(), root.root_status(), &file_id)
        })
        .await
}

/// Groups of files with identical content, most extra space first.
#[tauri::command]
pub async fn list_duplicates(
    state: State<'_, AppState>,
    query: DuplicateQuery,
) -> AtlasResult<DuplicatePage> {
    state
        .with_workspace(move |ws| {
            let root = ws.require_open()?;
            documents::duplicates::list(root.reader(), root.root_status(), &query)
        })
        .await
}

/// The documents closest in meaning to one document (Slice 2.5), best
/// first: never the document itself, its copies or a document with the
/// same text. Only stored vectors are compared: nothing is sent to an AI
/// service. The workspace is held only to take the library's handles.
#[tauri::command]
pub async fn get_related_documents(
    state: State<'_, AppState>,
    document_id: String,
    limit: Option<u32>,
) -> AtlasResult<RelatedPage> {
    let handles = state
        .with_workspace(|ws| {
            let root = ws.require_open()?;
            Ok((
                root.reader_handle(),
                root.root_status_handle(),
                std::sync::Arc::clone(root.vectors()),
                root.catalog().handle(),
            ))
        })
        .await?;
    tauri::async_runtime::spawn_blocking(move || {
        let (reader, roots, vectors, catalog) = handles;
        documents::related::run(
            &reader,
            &roots,
            &vectors,
            &catalog,
            &document_id,
            limit.unwrap_or(documents::related::DEFAULT_LIMIT),
        )
    })
    .await
    .map_err(|_| AtlasError::Internal("a background task failed"))?
}
