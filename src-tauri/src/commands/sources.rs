use crate::domain::dto::SourceRoot;
use crate::error::{AtlasError, AtlasResult};

#[tauri::command]
pub async fn list_source_roots() -> AtlasResult<Vec<SourceRoot>> {
    Err(AtlasError::not_implemented("list_source_roots", "0.3"))
}

#[tauri::command]
pub async fn add_source_root(path: String) -> AtlasResult<SourceRoot> {
    let _ = path;
    Err(AtlasError::not_implemented("add_source_root", "0.3"))
}

/// Removes a source root from Atlas. Never deletes files on disk.
#[tauri::command]
pub async fn remove_source_root(id: String) -> AtlasResult<()> {
    let _ = id;
    Err(AtlasError::not_implemented("remove_source_root", "0.3"))
}
