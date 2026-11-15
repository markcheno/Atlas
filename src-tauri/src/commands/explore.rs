//! Explore: the map of the library by meaning (Slice 4.6.1).

use tauri::State;

use crate::domain::dto::{ExploreMap, ExploreStatus};
use crate::error::{AtlasError, AtlasResult};
use crate::services::explore::app::{self, RenameError};
use crate::state::AppState;

/// Where the map stands: drawn or not, being drawn, how much has changed.
#[tauri::command]
pub async fn get_explore_status(state: State<'_, AppState>) -> AtlasResult<ExploreStatus> {
    state
        .with_workspace(|ws| {
            let root = ws.require_open()?;
            let shared = root.explore().clone();
            root.reader()
                .read(move |c| app::status(c, &shared))?
                .map_err(AtlasError::db("read the map's status"))
        })
        .await
}

/// The map: its areas and every book on it.
#[tauri::command]
pub async fn get_explore_map(state: State<'_, AppState>) -> AtlasResult<ExploreMap> {
    state
        .with_workspace(|ws| {
            let root = ws.require_open()?;
            let shared = root.explore().clone();
            root.reader()
                .read(move |c| app::map(c, &shared))?
                .map_err(AtlasError::db("read the map"))
        })
        .await
}

/// Draws the map again now (`fresh`: from scratch, not from the old map).
/// It runs in the background; `explore-status` says when it's done, or why
/// it waits.
#[tauri::command]
pub async fn refresh_explore_map(
    state: State<'_, AppState>,
    fresh: bool,
) -> AtlasResult<ExploreStatus> {
    state
        .with_workspace(move |ws| {
            let root = ws.require_open()?;
            root.explore().request(fresh);
            let shared = root.explore().clone();
            root.reader()
                .read(move |c| app::status(c, &shared))?
                .map_err(AtlasError::db("read the map's status"))
        })
        .await
}

/// The owner's name for an area or sub-area; `None` or blank goes back to
/// its words.
#[tauri::command]
pub async fn rename_area(
    state: State<'_, AppState>,
    area_id: i64,
    name: Option<String>,
) -> AtlasResult<()> {
    state
        .with_workspace(move |ws| {
            let root = ws.require_open()?;
            match root
                .catalog()
                .call(move |c| app::rename(c, area_id, name.as_deref()))?
                .map_err(AtlasError::db("rename the area"))?
            {
                Ok(()) => Ok(()),
                Err(RenameError::NotFound) => Err(AtlasError::AreaNotFound),
                Err(RenameError::TooLong) => Err(AtlasError::AreaNameTooLong),
            }
        })
        .await
}
