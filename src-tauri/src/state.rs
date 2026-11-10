//! Process-wide state managed by Tauri and shared by commands.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::domain::dto::{DeviceCheck, DeviceSnapshot};
use crate::domain::traits::SourceOpener;
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::http_probe::ProbeConfig;
use crate::services::embedding::query::QueryEmbedder;
use crate::services::providers::secrets::SecretVault;
use crate::services::system::ResourceGovernor;
use crate::services::workspace::Workspace;

pub struct AppState {
    pub workspace: Arc<Mutex<Workspace>>,
    /// Shared with the inventory worker so both see the same hysteresis.
    pub governor: Arc<Mutex<ResourceGovernor>>,
    /// Opens and reveals original files with the OS.
    pub opener: Arc<dyn SourceOpener>,
    /// API keys: the OS keychain, or this session only.
    pub vault: Arc<SecretVault>,
    pub probe: ProbeConfig,
    /// Embeds search queries for meaning search (Slice 2.4).
    pub query: Arc<QueryEmbedder>,
}

impl AppState {
    pub fn new(
        workspace: Workspace,
        governor: Arc<Mutex<ResourceGovernor>>,
        opener: Arc<dyn SourceOpener>,
        vault: Arc<SecretVault>,
        query: Arc<QueryEmbedder>,
    ) -> Self {
        Self {
            workspace: Arc::new(Mutex::new(workspace)),
            governor,
            opener,
            vault,
            probe: ProbeConfig::default(),
            query,
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

    async fn open_root_path(&self) -> AtlasResult<Option<PathBuf>> {
        self.with_workspace(|ws| Ok(ws.open_root().map(|r| r.path().to_path_buf())))
            .await
    }

    pub async fn device_snapshot(&self) -> AtlasResult<DeviceSnapshot> {
        let root = self.open_root_path().await?;
        let governor = Arc::clone(&self.governor);
        tauri::async_runtime::spawn_blocking(move || {
            let guard = governor
                .lock()
                .map_err(|_| AtlasError::Internal("resource state is unavailable"))?;
            Ok(guard.monitor.snapshot(root.as_deref()))
        })
        .await
        .map_err(|_| AtlasError::Internal("a background task failed"))?
    }

    pub async fn device_check(&self) -> AtlasResult<DeviceCheck> {
        let root = self.open_root_path().await?;
        let governor = Arc::clone(&self.governor);
        tauri::async_runtime::spawn_blocking(move || {
            let mut guard = governor
                .lock()
                .map_err(|_| AtlasError::Internal("resource state is unavailable"))?;
            Ok(guard.check(root.as_deref()))
        })
        .await
        .map_err(|_| AtlasError::Internal("a background task failed"))?
    }
}
