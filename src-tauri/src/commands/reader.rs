//! The page view (Slice 3.7, phase3-spec §5.5, §7.5, §9; ADR 0018).

use std::sync::Arc;

use tauri::State;

use crate::domain::dto::PageImage;
use crate::error::{AtlasError, AtlasResult};
use crate::services::reader;
use crate::state::AppState;

/// Page `pageNo` of a PDF, `widthPx` wide (at most 2,000), with `find`
/// (text the screen already shows) outlined when it is on the page. A file
/// only in the cloud needs `allowDownload`, as Open does. The workspace is
/// held only to look the file up; the page renders without it, in the
/// reader's own worker.
#[tauri::command]
pub async fn render_page(
    state: State<'_, AppState>,
    file_id: String,
    page_no: u32,
    width_px: u32,
    find: Option<String>,
    allow_download: Option<bool>,
) -> AtlasResult<PageImage> {
    let (service, prepared) = state
        .with_workspace(move |ws| {
            let root = ws.require_open()?;
            let prepared =
                reader::prepare(root, &file_id, page_no, allow_download.unwrap_or(false))?;
            Ok((Arc::clone(root.pages()), prepared))
        })
        .await?;
    tauri::async_runtime::spawn_blocking(move || {
        service.render(prepared, page_no, width_px, find.as_deref())
    })
    .await
    .map_err(|_| AtlasError::Internal("a background task failed"))?
}
