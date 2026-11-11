//! Interests, projects and suggestions (Slice 3.6, phase3-spec §5.4, §5.6,
//! §7.4, §9).

use std::sync::Arc;

use tauri::State;

use crate::domain::dto::{
    Collection, CollectionDetail, CollectionInput, CollectionItemRef, CollectionStatus, Membership,
    Suggestion,
};
use crate::error::{AtlasError, AtlasResult};
use crate::services::knowledge::{Recorder, collections, rediscovery};
use crate::state::AppState;

async fn with_recorder<R, F>(state: &State<'_, AppState>, f: F) -> AtlasResult<R>
where
    R: Send + 'static,
    F: FnOnce(&Recorder) -> AtlasResult<R> + Send + 'static,
{
    let recorder = state
        .with_workspace(|ws| Ok(Arc::clone(ws.require_open()?.recorder())))
        .await?;
    tauri::async_runtime::spawn_blocking(move || f(&recorder))
        .await
        .map_err(|_| AtlasError::Internal("a background task failed"))?
}

/// Interests and projects (all, or of one status), most recently changed
/// first.
#[tauri::command]
pub async fn list_collections(
    state: State<'_, AppState>,
    status: Option<CollectionStatus>,
) -> AtlasResult<Vec<Collection>> {
    with_recorder(&state, move |r| collections::list(r, status)).await
}

/// A collection's page: its trails, bookmarks, findings and documents.
#[tauri::command]
pub async fn get_collection(
    state: State<'_, AppState>,
    id: String,
) -> AtlasResult<CollectionDetail> {
    with_recorder(&state, move |r| collections::get(r, &id)).await
}

/// Creates (no id) or changes an interest or project.
#[tauri::command]
pub async fn save_collection(
    state: State<'_, AppState>,
    input: CollectionInput,
) -> AtlasResult<Collection> {
    with_recorder(&state, move |r| collections::save(r, &input)).await
}

/// Deletes an interest or project; what it held stays.
#[tauri::command]
pub async fn delete_collection(state: State<'_, AppState>, id: String) -> AtlasResult<()> {
    with_recorder(&state, move |r| collections::delete(r, &id)).await
}

#[tauri::command]
pub async fn add_to_collection(
    state: State<'_, AppState>,
    collection_id: String,
    item: CollectionItemRef,
) -> AtlasResult<Collection> {
    with_recorder(&state, move |r| collections::add(r, &collection_id, &item)).await
}

/// Takes an item out; the item stays.
#[tauri::command]
pub async fn remove_from_collection(
    state: State<'_, AppState>,
    collection_id: String,
    item: CollectionItemRef,
) -> AtlasResult<Collection> {
    with_recorder(&state, move |r| {
        collections::remove(r, &collection_id, &item)
    })
    .await
}

/// The collections an item is in, for Add to….
#[tauri::command]
pub async fn get_collection_membership(
    state: State<'_, AppState>,
    item: CollectionItemRef,
) -> AtlasResult<Membership> {
    with_recorder(&state, move |r| collections::membership(r, &item)).await
}

/// Suggestions from the library, from stored vectors only: no model is
/// called. None without a meaning index.
#[tauri::command]
pub async fn get_suggestions(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> AtlasResult<Vec<Suggestion>> {
    let (recorder, vectors, catalog) = state
        .with_workspace(|ws| {
            let root = ws.require_open()?;
            Ok((
                Arc::clone(root.recorder()),
                Arc::clone(root.vectors()),
                root.catalog().handle(),
            ))
        })
        .await?;
    tauri::async_runtime::spawn_blocking(move || {
        rediscovery::suggestions(&recorder, &vectors, &catalog, limit.unwrap_or(3) as usize)
    })
    .await
    .map_err(|_| AtlasError::Internal("a background task failed"))?
}

/// "Not for this": `seedId` is the collection, or empty for recent trails.
#[tauri::command]
pub async fn dismiss_suggestion(
    state: State<'_, AppState>,
    seed_id: String,
    document_id: String,
) -> AtlasResult<()> {
    with_recorder(&state, move |r| {
        rediscovery::dismiss(r, &seed_id, &document_id)
    })
    .await
}
