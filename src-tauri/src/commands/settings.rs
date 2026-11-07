use tauri::State;

use crate::domain::dto::Settings;
use crate::error::AtlasResult;
use crate::state::AppState;

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> AtlasResult<Settings> {
    state.with_workspace(|ws| ws.settings()).await
}

/// Replaces the settings and saves them to the open data root.
#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> AtlasResult<Settings> {
    state
        .with_workspace(move |ws| ws.update_settings(settings))
        .await
}
