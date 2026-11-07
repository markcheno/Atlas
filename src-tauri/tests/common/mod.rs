//! Shared helpers for integration tests: temporary data roots and config
//! folders, and a fake volume inspector.
#![allow(dead_code)] // Each test crate uses a different subset.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_lib::domain::dto::VolumeKind;
use atlas_lib::domain::traits::{VolumeInfo, VolumeInspector};
use atlas_lib::error::{AtlasResult, ErrorCode};
use atlas_lib::services::workspace::{Workspace, WorkspaceConfig};

pub const GIB: u64 = 1024 * 1024 * 1024;

pub struct FakeVolumes {
    pub kind: VolumeKind,
    pub free: u64,
    pub total: u64,
}

impl VolumeInspector for FakeVolumes {
    fn inspect(&self, _path: &Path) -> AtlasResult<VolumeInfo> {
        Ok(VolumeInfo {
            free_bytes: Some(self.free),
            total_bytes: Some(self.total),
            fs_type: Some("fake".into()),
            kind: self.kind,
        })
    }
}

pub fn local() -> Arc<FakeVolumes> {
    Arc::new(FakeVolumes {
        kind: VolumeKind::Local,
        free: 100 * GIB,
        total: 500 * GIB,
    })
}

pub struct Env {
    _tmp: tempfile::TempDir,
    pub config: PathBuf,
    pub base: PathBuf,
}

impl Env {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("config");
        let base = tmp.path().join("data");
        fs::create_dir_all(&base).unwrap();
        Self {
            config,
            base,
            _tmp: tmp,
        }
    }

    pub fn workspace(&self) -> Workspace {
        self.workspace_with(local())
    }

    pub fn workspace_with(&self, volumes: Arc<dyn VolumeInspector>) -> Workspace {
        Workspace::new(
            WorkspaceConfig {
                config_dir: self.config.clone(),
                default_root: self.config.join("Library"),
            },
            volumes,
        )
    }

    /// A second, unrelated Atlas install (its own locator).
    pub fn other_workspace(&self) -> Workspace {
        Workspace::new(
            WorkspaceConfig {
                config_dir: self.base.join("other-config"),
                default_root: self.base.join("other-default"),
            },
            local(),
        )
    }
}

pub fn code(result: AtlasResult<impl std::fmt::Debug>) -> ErrorCode {
    result.expect_err("expected an error").code()
}

pub fn dir_listing(path: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
