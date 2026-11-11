//! Ask Atlas (Slice 2.6, phase2-spec §7.6, ADR 0015).

use std::sync::Arc;

use rusqlite::OptionalExtension;
use tauri::{AppHandle, Emitter, State};

use crate::domain::dto::{
    Acknowledgement, AnswerEvent, AskQuery, AskReply, ProviderLocation, Settings,
};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::now_rfc3339;
use crate::services::answers::stream::ChatLimits;
use crate::services::answers::{self, AnswerEvents, AskDeps};
use crate::services::providers::{privacy, url_policy};
use crate::services::search::MeaningContext;
use crate::state::AppState;

/// Answers a question from the library. Returns the sources and an answer
/// id whose `answer-stream` events follow; or that the library has
/// nothing on it; or, for an internet service not agreed to yet, what
/// would be sent (nothing is). Only the passages found are sent, to the
/// chat role's service, after this explicit action. While recording is
/// on, the question and its answer become a step of the user's trail
/// when the answer ends (Slice 3.3).
#[tauri::command]
pub async fn ask_atlas(
    app: AppHandle,
    state: State<'_, AppState>,
    query: AskQuery,
) -> AtlasResult<AskReply> {
    let handles = state
        .with_workspace(|ws| {
            let root = ws.require_open()?;
            Ok((
                root.reader_handle(),
                Arc::clone(root.vectors()),
                root.catalog().handle(),
                root.settings().privacy.clone(),
                Arc::clone(root.answers()),
                Arc::clone(root.recorder()),
            ))
        })
        .await?;
    let (vault, embedder) = (state.vault.clone(), state.query.clone());
    let events: AnswerEvents = Arc::new(move |event: &AnswerEvent| {
        let _ = app.emit(crate::ANSWER_EVENT, event);
    });
    tauri::async_runtime::spawn_blocking(move || {
        let (reader, vectors, catalog, privacy_settings, registry, recorder) = handles;
        answers::ask(
            &AskDeps {
                reader: &reader,
                meaning: MeaningContext {
                    vectors: &vectors,
                    catalog: &catalog,
                    embedder: &embedder,
                    vault: &vault,
                    allow_remote: privacy::remote_embedding(&privacy_settings),
                },
                vault: &vault,
                privacy: &privacy_settings,
                locate: url_policy::location_of,
                limits: ChatLimits::default(),
                events,
                answers: &registry,
                recorder: Some(&recorder),
            },
            &query,
        )
    })
    .await
    .map_err(|_| AtlasError::Internal("a background task failed"))?
}

/// Stops the answer `answerId`; nothing if it already ended. The text so
/// far stays on screen; nothing is kept anywhere else.
#[tauri::command]
pub async fn cancel_answer(state: State<'_, AppState>, answer_id: String) -> AtlasResult<()> {
    state
        .with_workspace(move |ws| {
            ws.require_open()?.answers().cancel(&answer_id);
            Ok(())
        })
        .await
}

/// Records that the user saw what Ask Atlas sends to this internet
/// service and agreed, for this profile at its current address. Returns
/// the settings saved.
#[tauri::command]
pub async fn acknowledge_disclosure(
    state: State<'_, AppState>,
    profile_id: String,
) -> AtlasResult<Settings> {
    state
        .with_workspace(move |ws| {
            let root = ws.require_open()?;
            let id = profile_id.clone();
            let profile: Option<(String, String)> = root
                .reader()
                .read(move |c| {
                    c.query_row(
                        "SELECT label, base_url FROM provider_profiles WHERE id = ?1",
                        [&id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                })?
                .map_err(AtlasError::db("read AI services"))?;
            let (label, base_url) = profile.ok_or(AtlasError::ProfileNotFound)?;
            let mut settings = root.settings().clone();
            if url_policy::location_of(&base_url) != ProviderLocation::Internet {
                return Ok(settings); // Nothing to agree to.
            }
            privacy::acknowledge(
                &mut settings.privacy,
                Acknowledgement {
                    profile_id,
                    base_url,
                    label,
                    at: now_rfc3339(),
                },
            );
            ws.update_settings(settings)
        })
        .await
}
