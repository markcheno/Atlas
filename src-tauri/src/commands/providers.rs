use tauri::State;

use crate::domain::dto::{
    ConnectionResult, ConnectionTestTarget, LocalProviderSuggestion, ProviderProfile,
    ProviderProfileInput,
};
use crate::domain::traits::ModelProvider;
use crate::error::AtlasResult;
use crate::infrastructure::http_probe::HttpModelProvider;
use crate::services::providers::detect;
use crate::services::providers::profiles::{self, Prepared};
use crate::state::AppState;

#[tauri::command]
pub async fn list_provider_profiles(
    state: State<'_, AppState>,
) -> AtlasResult<Vec<ProviderProfile>> {
    let vault = state.vault.clone();
    state
        .with_workspace(move |ws| profiles::list(ws.require_open()?, &vault))
        .await
}

/// Creates or updates a profile; an included API key goes to the keychain.
#[tauri::command]
pub async fn save_provider_profile(
    state: State<'_, AppState>,
    profile: ProviderProfileInput,
) -> AtlasResult<ProviderProfile> {
    let vault = state.vault.clone();
    state
        .with_workspace(move |ws| profiles::save(ws.require_open()?, &vault, profile))
        .await
}

/// Removes a profile and its stored key.
#[tauri::command]
pub async fn delete_provider_profile(state: State<'_, AppState>, id: String) -> AtlasResult<()> {
    let vault = state.vault.clone();
    state
        .with_workspace(move |ws| profiles::delete(ws.require_open()?, &vault, &id))
        .await
}

/// Asks the service for its model list, after explicit user action only.
/// Nothing from the library is sent.
#[tauri::command]
pub async fn test_provider_connection(
    state: State<'_, AppState>,
    target: ConnectionTestTarget,
) -> AtlasResult<ConnectionResult> {
    let vault = state.vault.clone();
    let prepared = state
        .with_workspace(move |ws| profiles::prepare_test(ws.require_open()?, &vault, &target))
        .await?;
    Ok(match prepared {
        Prepared::Blocked(result) => result,
        Prepared::Ready {
            kind,
            endpoint,
            key,
        } => {
            let provider = HttpModelProvider {
                kind,
                config: state.probe,
            };
            provider
                .test_connection(&endpoint.base, key.as_ref().map(|k| k.expose()))
                .await
        }
    })
}

/// Looks for AI services on this computer's well-known ports. Runs only
/// when the user asks.
#[tauri::command]
pub async fn detect_local_providers() -> AtlasResult<Vec<LocalProviderSuggestion>> {
    Ok(detect::detect(detect::KNOWN_PORTS, detect::config()).await)
}
