use tauri::State;

use crate::domain::dto::{SearchIntent, SearchPage, SearchQuery};
use crate::error::{AtlasError, AtlasResult};
use crate::services::diagnostics::Event;
use crate::services::diagnostics::event::MeaningUse;
use crate::services::search::{self, MeaningContext};
use crate::state::AppState;

/// Search over the text Atlas has read and, when an embedding model is
/// set up, over its meaning, grouped by document, best match first
/// (phase1-spec §7.4, phase2-spec §7.4). The workspace is held only to
/// take the library's handles: a query's embedding (up to 1.5 s) never
/// blocks other commands.
///
/// `intent` says why it ran (Slice 3.3): only a `submit` is recorded into
/// the user's trail; paging, a live refresh and a filter change are
/// `refresh`, the default.
#[tauri::command]
pub async fn search_documents(
    state: State<'_, AppState>,
    query: SearchQuery,
    intent: Option<SearchIntent>,
) -> AtlasResult<SearchPage> {
    let handles = state
        .with_workspace(|ws| {
            let root = ws.require_open()?;
            Ok((
                root.reader_handle(),
                root.root_status_handle(),
                std::sync::Arc::clone(root.vectors()),
                root.catalog().handle(),
                crate::services::providers::privacy::remote_embedding(&root.settings().privacy),
                std::sync::Arc::clone(root.recorder()),
                root.diagnostics().clone(),
            ))
        })
        .await?;
    let (vault, embedder) = (state.vault.clone(), state.query.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let (reader, roots, vectors, catalog, allow_remote, recorder, diagnostics) = handles;
        let started = std::time::Instant::now();
        let page = search::run_hybrid(
            &reader,
            &roots,
            &MeaningContext {
                vectors: &vectors,
                catalog: &catalog,
                embedder: &embedder,
                vault: &vault,
                allow_remote,
            },
            &query,
        )?;
        diagnostics.record(Event::Search {
            ms: started.elapsed().as_millis() as u64,
            hits: page.hits.len() as u32,
            matched: page.matched_documents,
            meaning: MeaningUse::from(&page.semantic),
        });
        recorder.search(&query, intent.unwrap_or_default(), &page);
        Ok(page)
    })
    .await
    .map_err(|_| AtlasError::Internal("a background task failed"))?
}
