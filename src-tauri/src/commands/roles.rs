//! Model roles, capability tests and recommendations (Slice 2.1,
//! phase2-spec §7.1, ADR 0012).

use tauri::State;

use crate::domain::dto::{
    AiRole, CapabilityOutcome, CapabilityResult, ConnectionTestTarget, ModelRoles, Recommendations,
};
use crate::domain::traits::ModelProvider;
use crate::error::AtlasResult;
use crate::infrastructure::clock::now_rfc3339;
use crate::infrastructure::http_probe::{HttpModelProvider, ProbeConfig};
use crate::services::embedding;
use crate::services::providers::profiles::{self, Prepared};
use crate::services::providers::{recommend, roles};
use crate::state::AppState;

/// Sends one fixed request to `model_id` to check it can do `role`, after
/// explicit user action only. Nothing from the library is sent.
#[tauri::command]
pub async fn test_model_capability(
    state: State<'_, AppState>,
    target: ConnectionTestTarget,
    role: AiRole,
    model_id: String,
) -> AtlasResult<CapabilityResult> {
    let vault = state.vault.clone();
    let prepared = state
        .with_workspace(move |ws| profiles::resolve(ws.require_open()?, &vault, &target))
        .await?;
    Ok(match prepared {
        Prepared::Blocked(message) => CapabilityResult {
            role,
            model_id,
            outcome: CapabilityOutcome::InsecureUrlRejected,
            dimensions: None,
            message: message.to_owned(),
            elapsed_ms: 0,
            tested_at: now_rfc3339(),
        },
        Prepared::Ready {
            kind,
            endpoint,
            key,
        } => {
            let provider = HttpModelProvider {
                kind,
                config: ProbeConfig::capability(),
            };
            provider
                .test_capability(
                    &endpoint.base,
                    key.as_ref().map(|k| k.expose()),
                    role,
                    &model_id,
                )
                .await
        }
    })
}

#[tauri::command]
pub async fn get_model_roles(state: State<'_, AppState>) -> AtlasResult<ModelRoles> {
    state
        .with_workspace(|ws| roles::get(ws.require_open()?))
        .await
}

/// Assigns a role; `result` must be a recent passing capability test of
/// that model for that role.
#[tauri::command]
pub async fn set_model_role(
    state: State<'_, AppState>,
    role: AiRole,
    profile_id: String,
    model_id: String,
    result: CapabilityResult,
) -> AtlasResult<ModelRoles> {
    state
        .with_workspace(move |ws| {
            let root = ws.require_open()?;
            let roles = roles::set(root, role, &profile_id, &model_id, &result)?;
            embedding::triggers::changed(root)?;
            // A first embedding model in a catalog with 4 KiB pages: reopen
            // the library, which converts it before anything is embedded
            // (the UI says it is preparing while this command runs).
            if role == AiRole::Embedding && ws.wants_page_conversion() {
                ws.reopen();
                ws.require_open()?;
            }
            Ok(roles)
        })
        .await
}

#[tauri::command]
pub async fn clear_model_role(state: State<'_, AppState>, role: AiRole) -> AtlasResult<ModelRoles> {
    state
        .with_workspace(move |ws| {
            let root = ws.require_open()?;
            let roles = roles::clear(root, role)?;
            embedding::triggers::changed(root)?;
            Ok(roles)
        })
        .await
}

/// From the bundled catalog and this computer's memory. `listed_model_ids`
/// are ids the UI already has from this session's tests; no network.
#[tauri::command]
pub async fn get_model_recommendations(
    state: State<'_, AppState>,
    listed_model_ids: Vec<String>,
) -> AtlasResult<Recommendations> {
    let device = state.device_snapshot().await?;
    Ok(recommend::recommend(
        &recommend::bundled(),
        &device,
        &listed_model_ids,
    ))
}
