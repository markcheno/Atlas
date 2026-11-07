//! Process-wide state managed by Tauri and shared by commands.

use std::sync::{Arc, Mutex};

use crate::error::{AtlasError, AtlasResult};
use crate::services::workspace::Workspace;

pub struct AppState {
    pub workspace: Arc<Mutex<Workspace>>,
}

impl AppState {
    pub fn new(workspace: Workspace) -> Self {
        Self {
            workspace: Arc::new(Mutex::new(workspace)),
        }
    }

    /// Runs `f` against the workspace on a blocking thread. Workspace
    /// operations do filesystem and SQLite I/O, which must not run on the
    /// async executor.
    pub async fn with_workspace<R, F>(&self, f: F) -> AtlasResult<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Workspace) -> AtlasResult<R> + Send + 'static,
    {
        let workspace = Arc::clone(&self.workspace);
        tauri::async_runtime::spawn_blocking(move || {
            let mut guard = workspace
                .lock()
                .map_err(|_| AtlasError::Internal("workspace state is unavailable"))?;
            f(&mut guard)
        })
        .await
        .map_err(|_| AtlasError::Internal("a background task failed"))?
    }
}
