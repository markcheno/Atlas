use crate::error::{AtlasError, AtlasResult};

/// Creates a new Atlas Data Root at `path`, or reopens a compatible one.
#[tauri::command]
pub async fn choose_data_root(path: String) -> AtlasResult<()> {
    let _ = path;
    Err(AtlasError::not_implemented("choose_data_root", "0.2"))
}

/// Opens an existing Atlas Data Root at `path` without creating anything.
#[tauri::command]
pub async fn open_data_root(path: String) -> AtlasResult<()> {
    let _ = path;
    Err(AtlasError::not_implemented("open_data_root", "0.2"))
}
