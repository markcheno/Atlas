use crate::domain::dto::{DocumentPage, DocumentQuery};
use crate::error::{AtlasError, AtlasResult};

/// Paginated filename/path listing of cataloged files.
#[tauri::command]
pub async fn list_documents(query: DocumentQuery) -> AtlasResult<DocumentPage> {
    let _ = query;
    Err(AtlasError::not_implemented("list_documents", "0.6"))
}

/// Opens a cataloged file in the OS default application.
#[tauri::command]
pub async fn open_original(file_id: String) -> AtlasResult<()> {
    let _ = file_id;
    Err(AtlasError::not_implemented("open_original", "0.6"))
}

/// Reveals a cataloged file in the OS file manager.
#[tauri::command]
pub async fn reveal_original(file_id: String) -> AtlasResult<()> {
    let _ = file_id;
    Err(AtlasError::not_implemented("reveal_original", "0.6"))
}
