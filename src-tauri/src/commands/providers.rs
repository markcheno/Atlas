use crate::domain::dto::{
    ConnectionResult, ConnectionTestTarget, ProviderProfile, ProviderProfileInput,
};
use crate::error::{AtlasError, AtlasResult};

#[tauri::command]
pub async fn list_provider_profiles() -> AtlasResult<Vec<ProviderProfile>> {
    Err(AtlasError::not_implemented("list_provider_profiles", "0.7"))
}

#[tauri::command]
pub async fn save_provider_profile(profile: ProviderProfileInput) -> AtlasResult<ProviderProfile> {
    let _ = profile;
    Err(AtlasError::not_implemented("save_provider_profile", "0.7"))
}

/// Probes an endpoint's model listing after explicit user action only.
#[tauri::command]
pub async fn test_provider_connection(
    target: ConnectionTestTarget,
) -> AtlasResult<ConnectionResult> {
    let _ = target;
    Err(AtlasError::not_implemented(
        "test_provider_connection",
        "0.7",
    ))
}
