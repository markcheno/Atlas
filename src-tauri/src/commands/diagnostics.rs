//! Diagnostics (Slice 4.1, phase4-spec §5.8, §7.1; ADR 0021): the log's
//! size, saving a report, showing either folder, and clearing the log. The
//! renderer sends nothing but which folder to show.

use tauri::State;

use crate::domain::dto::{DiagnosticsFolder, DiagnosticsStatus, ReportSaved};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::now_rfc3339;
use crate::services::diagnostics::report::{self, Inputs, KnowledgeSummary};
use crate::services::diagnostics::{self, Diagnostics, Event, LOGS_DIR};
use crate::services::storage;
use crate::state::AppState;

async fn blocking<R: Send + 'static>(
    f: impl FnOnce() -> AtlasResult<R> + Send + 'static,
) -> AtlasResult<R> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| AtlasError::Internal("a background task failed"))?
}

async fn open(state: &State<'_, AppState>) -> AtlasResult<(std::path::PathBuf, Diagnostics)> {
    state
        .with_workspace(|ws| {
            let root = ws.require_open()?;
            Ok((root.path().to_path_buf(), root.diagnostics().clone()))
        })
        .await
}

#[tauri::command]
pub async fn get_diagnostics_status(state: State<'_, AppState>) -> AtlasResult<DiagnosticsStatus> {
    let (root, d) = open(&state).await?;
    blocking(move || Ok(diagnostics::status(&root, &d))).await
}

/// Writes a report into the library's `exports/diagnostics`. Works with the
/// knowledge store damaged: the report then gives its state only.
#[tauri::command]
pub async fn save_diagnostics_report(state: State<'_, AppState>) -> AtlasResult<ReportSaved> {
    let (root, volumes, settings, knowledge, d) = state
        .with_workspace(|ws| {
            let root = ws.require_open()?;
            let store = root.knowledge();
            let knowledge = KnowledgeSummary {
                state: store.state(),
                status: (store.state() == crate::domain::dto::KnowledgeState::Ready)
                    .then(|| store.status()),
            };
            Ok((
                root.path().to_path_buf(),
                ws.volumes(),
                root.settings().clone(),
                knowledge,
                root.diagnostics().clone(),
            ))
        })
        .await?;
    let device = state.device_check().await.ok();
    blocking(move || {
        d.flush();
        let saved_at = now_rfc3339();
        let inputs = Inputs {
            storage: storage::usage(&root, volumes.as_ref()).ok(),
            root: root.clone(),
            device,
            settings,
            knowledge,
            dropped: d.dropped(),
            saved_at: saved_at.clone(),
        };
        let saved = report::save(&inputs)?;
        d.record(Event::ReportSaved { bytes: saved.bytes });
        d.flush();
        Ok(ReportSaved {
            saved_at,
            bytes: saved.bytes,
            status: diagnostics::status(&root, &d),
        })
    })
    .await
}

/// Shows the reports' folder or the log's in the file manager.
#[tauri::command]
pub async fn reveal_diagnostics(
    state: State<'_, AppState>,
    folder: DiagnosticsFolder,
) -> AtlasResult<()> {
    let opener = state.opener.clone();
    let (root, _) = open(&state).await?;
    blocking(move || {
        let target = match folder {
            DiagnosticsFolder::Reports => {
                let newest = report::saved(&root).pop().map(|(path, _)| path);
                newest.ok_or(AtlasError::NoDiagnosticsReport)?
            }
            DiagnosticsFolder::Log => root.join(LOGS_DIR),
        };
        opener.reveal_original(&target)
    })
    .await
}

/// Removes the log's files. The log goes on in a new file.
#[tauri::command]
pub async fn clear_log(state: State<'_, AppState>) -> AtlasResult<DiagnosticsStatus> {
    let (root, d) = open(&state).await?;
    blocking(move || {
        d.clear();
        Ok(diagnostics::status(&root, &d))
    })
    .await
}
