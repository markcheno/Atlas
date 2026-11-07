//! Slice 0.2: Atlas Data Root lifecycle (phase0-spec §6; tests P0-02,
//! P0-03, P0-04, P0-14, P0-23 plus creation-safety cases).
//!
//! Every test uses temporary directories for both the data root and the
//! locator's config folder, and a fake volume inspector, so nothing touches
//! the real user configuration.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_lib::domain::dto::{
    AppearanceSettings, CandidateKind, CandidateProblem, DataRootState, Settings, ThemePreference,
    VolumeKind,
};
use atlas_lib::domain::traits::{VolumeInfo, VolumeInspector};
use atlas_lib::error::{AtlasResult, ErrorCode};
use atlas_lib::services::catalog;
use atlas_lib::services::workspace::layout::{
    CATALOG, DIRECTORIES, INIT_MARKER, MANIFEST, SUGGESTED_SUBFOLDER,
};
use atlas_lib::services::workspace::{Workspace, WorkspaceConfig, manifest};

const GIB: u64 = 1024 * 1024 * 1024;

struct FakeVolumes {
    kind: VolumeKind,
    free: u64,
    total: u64,
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

fn local() -> Arc<FakeVolumes> {
    Arc::new(FakeVolumes {
        kind: VolumeKind::Local,
        free: 100 * GIB,
        total: 500 * GIB,
    })
}

struct Env {
    _tmp: tempfile::TempDir,
    config: PathBuf,
    base: PathBuf,
}

impl Env {
    fn new() -> Self {
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

    fn workspace(&self) -> Workspace {
        self.workspace_with(local())
    }

    fn workspace_with(&self, volumes: Arc<dyn VolumeInspector>) -> Workspace {
        Workspace::new(
            WorkspaceConfig {
                config_dir: self.config.clone(),
                default_root: self.config.join("Library"),
            },
            volumes,
        )
    }

    /// A second, unrelated Atlas install (its own locator).
    fn other_workspace(&self) -> Workspace {
        Workspace::new(
            WorkspaceConfig {
                config_dir: self.base.join("other-config"),
                default_root: self.base.join("other-default"),
            },
            local(),
        )
    }
}

fn code(result: AtlasResult<impl std::fmt::Debug>) -> ErrorCode {
    result.expect_err("expected an error").code()
}

fn dir_listing(path: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn first_run_is_not_configured() {
    let env = Env::new();
    let mut ws = env.workspace();
    ws.startup();
    assert_eq!(ws.status().state, DataRootState::NotConfigured);
    assert_eq!(code(ws.settings()), ErrorCode::NoDataRoot);
    assert!(
        !env.config.exists(),
        "startup must not create the config dir"
    );
}

/// P0-02: create, close, reopen the same catalog and settings.
#[test]
fn p0_02_create_then_reopen_after_restart() {
    let env = Env::new();
    let root = env.base.join("Atlas Library");

    let root_id = {
        let mut ws = env.workspace();
        ws.startup();
        let status = ws.choose(&root).unwrap();
        assert_eq!(status.state, DataRootState::Open);
        assert_eq!(status.display_path.as_deref(), root.to_str());
        ws.update_settings(Settings {
            appearance: AppearanceSettings {
                theme: ThemePreference::Dark,
            },
        })
        .unwrap();
        status.root_id.unwrap()
    };

    for dir in DIRECTORIES {
        assert!(root.join(dir).is_dir(), "missing {dir}");
    }
    assert!(!root.join(INIT_MARKER).exists());
    let m = manifest::read(&root).unwrap();
    assert_eq!(m.root_id, root_id);
    assert_eq!(m.catalog_schema_version, catalog::SUPPORTED_SCHEMA_VERSION);

    let mut ws = env.workspace();
    ws.startup();
    let status = ws.status();
    assert_eq!(status.state, DataRootState::Open);
    assert_eq!(status.root_id.as_deref(), Some(root_id.as_str()));
    assert_eq!(
        ws.settings().unwrap().appearance.theme,
        ThemePreference::Dark
    );

    // The catalog's workspace row carries the same ID.
    let stored: String = ws
        .open_root()
        .unwrap()
        .catalog()
        .call(|c| c.query_row("SELECT id FROM workspace", [], |r| r.get(0)))
        .unwrap()
        .unwrap();
    assert_eq!(stored, root_id);
}

/// P0-03: a catalog from a newer Atlas is refused and left byte-identical.
#[test]
fn p0_03_newer_schema_is_refused_without_writes() {
    let env = Env::new();
    let root = env.base.join("lib");
    env.workspace().choose(&root).unwrap();

    {
        let conn = rusqlite::Connection::open(root.join(CATALOG)).unwrap();
        conn.execute(
            "INSERT INTO schema_migrations VALUES (999, 'from the future', 't')",
            [],
        )
        .unwrap();
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
    }
    let catalog_before = fs::read(root.join(CATALOG)).unwrap();
    let manifest_before = fs::read(root.join(MANIFEST)).unwrap();

    let mut ws = env.workspace();
    ws.startup();
    let status = ws.status();
    assert_eq!(status.state, DataRootState::Incompatible);
    assert_eq!(
        status.problem.unwrap().code,
        ErrorCode::DataRootIncompatible
    );
    assert_eq!(code(ws.open(&root)), ErrorCode::DataRootIncompatible);

    assert_eq!(fs::read(root.join(CATALOG)).unwrap(), catalog_before);
    assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), manifest_before);
}

/// P0-04: a second writer is blocked until the first closes.
#[test]
fn p0_04_second_process_is_blocked_by_the_lock() {
    let env = Env::new();
    let root = env.base.join("lib");
    let mut first = env.workspace();
    first.choose(&root).unwrap();

    let mut second = env.other_workspace();
    assert_eq!(code(second.open(&root)), ErrorCode::DataRootInUse);

    // A restart of the same install while another copy runs reports in_use.
    let mut restarted = env.workspace();
    restarted.startup();
    assert_eq!(restarted.status().state, DataRootState::InUse);

    drop(first);
    assert_eq!(second.open(&root).unwrap().state, DataRootState::Open);
}

/// P0-14: a missing data drive is reported, never recreated.
#[test]
fn p0_14_missing_data_root_is_not_recreated() {
    let env = Env::new();
    let drive = env.base.join("ExternalDrive");
    let root = drive.join("Atlas Library");
    env.workspace().choose(&root).unwrap();

    // Simulate unplugging the drive.
    fs::rename(&drive, env.base.join("unplugged")).unwrap();

    let mut ws = env.workspace();
    ws.startup();
    let status = ws.status();
    assert_eq!(status.state, DataRootState::Missing);
    assert_eq!(status.problem.unwrap().code, ErrorCode::DataRootMissing);
    assert_eq!(code(ws.open(&root)), ErrorCode::DataRootMissing);
    assert!(!drive.exists(), "Atlas must not recreate a missing root");

    // Reconnecting the drive makes it openable again.
    fs::rename(env.base.join("unplugged"), &drive).unwrap();
    assert_eq!(ws.open(&root).unwrap().state, DataRootState::Open);
}

/// P0-23: network and cloud-synced volumes are refused before any write.
#[test]
fn p0_23_network_and_cloud_volumes_are_refused() {
    let env = Env::new();
    for (kind, problem, expected) in [
        (
            VolumeKind::Network,
            CandidateProblem::NetworkVolume,
            ErrorCode::DataRootNetworkVolume,
        ),
        (
            VolumeKind::CloudSynced,
            CandidateProblem::CloudSynced,
            ErrorCode::DataRootCloudSynced,
        ),
    ] {
        let mut ws = env.workspace_with(Arc::new(FakeVolumes {
            kind,
            free: 100 * GIB,
            total: 500 * GIB,
        }));
        let root = env.base.join(format!("{kind:?}"));
        let candidate = ws.inspect(&root).unwrap();
        assert!(candidate.problems.contains(&problem));
        assert!(!candidate.can_use);
        assert_eq!(code(ws.choose(&root)), expected);
        assert!(!root.exists());
    }
}

#[test]
fn interrupted_creation_is_resumed_with_a_stable_id() {
    let env = Env::new();
    let root = env.base.join("lib");
    fs::create_dir_all(root.join("library")).unwrap();
    fs::write(root.join(INIT_MARKER), "0.2.0").unwrap();
    let (writer, _) = catalog::open(&root.join(CATALOG), "t".into()).unwrap();
    let earlier_id = catalog::ensure_workspace(
        &writer,
        "8a3e2a39-6c39-4f5c-8f55-7f3f0bdfa001".into(),
        "My Library".into(),
        "t".into(),
    )
    .unwrap();
    drop(writer);

    let mut ws = env.workspace();
    assert_eq!(
        ws.inspect(&root).unwrap().kind,
        CandidateKind::IncompleteLibrary
    );
    // Opening is refused; only choosing (creating) finishes the job.
    assert_eq!(code(ws.open(&root)), ErrorCode::DataRootInvalid);

    let status = ws.choose(&root).unwrap();
    assert_eq!(status.root_id.as_deref(), Some(earlier_id.as_str()));
    assert!(!root.join(INIT_MARKER).exists());
    assert!(root.join(MANIFEST).is_file());
}

#[test]
fn non_empty_folder_is_refused_and_a_subfolder_offered() {
    let env = Env::new();
    let docs = env.base.join("Documents");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join("thesis.pdf"), b"%PDF").unwrap();
    fs::write(docs.join(".DS_Store"), b"").unwrap();
    let before = dir_listing(&docs);

    let mut ws = env.workspace();
    let candidate = ws.inspect(&docs).unwrap();
    assert_eq!(candidate.kind, CandidateKind::NonEmptyFolder);
    assert!(!candidate.can_use);
    let suggested = candidate.suggested_subfolder.unwrap();
    assert_eq!(PathBuf::from(&suggested), docs.join(SUGGESTED_SUBFOLDER));

    assert_eq!(code(ws.choose(&docs)), ErrorCode::FolderNotEmpty);
    assert_eq!(dir_listing(&docs), before, "nothing written");

    assert_eq!(
        ws.choose(Path::new(&suggested)).unwrap().state,
        DataRootState::Open
    );
    assert_eq!(fs::read(docs.join("thesis.pdf")).unwrap(), b"%PDF");
}

#[test]
fn os_clutter_does_not_make_a_folder_non_empty() {
    let env = Env::new();
    let dir = env.base.join("empty");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(".DS_Store"), b"").unwrap();
    assert_eq!(
        env.workspace().inspect(&dir).unwrap().kind,
        CandidateKind::EmptyFolder
    );
}

#[test]
fn low_space_refuses_new_libraries_but_opens_existing_ones() {
    let env = Env::new();
    let existing = env.base.join("existing");
    env.workspace().choose(&existing).unwrap();

    let mut ws = env.workspace_with(Arc::new(FakeVolumes {
        kind: VolumeKind::Local,
        free: GIB,
        total: 100 * GIB,
    }));
    let new_root = env.base.join("new");
    let candidate = ws.inspect(&new_root).unwrap();
    assert!(candidate.problems.contains(&CandidateProblem::LowSpace));
    assert_eq!(candidate.required_free_bytes, Some(2 * GIB));
    let err = ws.choose(&new_root).unwrap_err();
    assert_eq!(err.code(), ErrorCode::DataRootLowSpace);
    assert!(err.to_string().contains("1024 MiB free"), "{err}");
    assert!(!new_root.exists());

    assert_eq!(ws.choose(&existing).unwrap().state, DataRootState::Open);
}

#[cfg(unix)]
#[test]
fn unwritable_location_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    let parent = env.base.join("readonly");
    fs::create_dir_all(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();

    let target = parent.join("lib");
    let mut ws = env.workspace();
    let candidate = ws.inspect(&target).unwrap();
    let result = ws.choose(&target);
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(candidate.problems.contains(&CandidateProblem::NotWritable));
    assert_eq!(code(result), ErrorCode::DataRootNotWritable);
    assert!(!target.exists());
}

#[test]
fn damaged_manifest_is_reported_and_left_untouched() {
    let env = Env::new();
    let root = env.base.join("lib");
    env.workspace().choose(&root).unwrap();
    fs::write(root.join(MANIFEST), b"{ not json").unwrap();

    let mut ws = env.workspace();
    ws.startup();
    assert_eq!(ws.status().state, DataRootState::Invalid);
    assert_eq!(
        ws.inspect(&root).unwrap().problems,
        vec![CandidateProblem::InvalidManifest]
    );
    assert_eq!(code(ws.choose(&root)), ErrorCode::DataRootInvalid);
    assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), b"{ not json");
}

#[test]
fn library_inside_a_library_is_refused() {
    let env = Env::new();
    let root = env.base.join("lib");
    let mut ws = env.workspace();
    ws.choose(&root).unwrap();
    let nested = root.join("exports").join("nested");
    assert_eq!(code(ws.choose(&nested)), ErrorCode::DataRootNested);
    assert!(!nested.exists());
}

#[test]
fn a_file_is_not_a_folder() {
    let env = Env::new();
    let file = env.base.join("file.txt");
    fs::write(&file, b"x").unwrap();
    let mut ws = env.workspace();
    assert_eq!(ws.inspect(&file).unwrap().kind, CandidateKind::NotAFolder);
    assert_eq!(code(ws.choose(&file)), ErrorCode::DataRootNotAFolder);
}

#[test]
fn relative_paths_are_rejected() {
    let env = Env::new();
    let mut ws = env.workspace();
    assert_eq!(
        code(ws.choose(Path::new("relative/lib"))),
        ErrorCode::InvalidPath
    );
}

#[test]
fn staging_files_are_cleaned_on_open() {
    let env = Env::new();
    let root = env.base.join("lib");
    env.workspace().choose(&root).unwrap();
    fs::write(root.join("tmp").join(".leftover.tmp"), b"partial").unwrap();

    let mut ws = env.workspace();
    ws.startup();
    assert_eq!(ws.status().state, DataRootState::Open);
    assert!(dir_listing(&root.join("tmp")).is_empty());
}

#[test]
fn choosing_the_open_root_again_is_a_no_op() {
    let env = Env::new();
    let root = env.base.join("lib");
    let mut ws = env.workspace();
    let first = ws.choose(&root).unwrap();
    assert_eq!(ws.choose(&root).unwrap(), first);
    assert_eq!(ws.open(&root).unwrap(), first);
}

#[test]
fn switching_roots_releases_the_previous_lock() {
    let env = Env::new();
    let a = env.base.join("a");
    let b = env.base.join("b");
    let mut ws = env.workspace();
    ws.choose(&a).unwrap();
    ws.choose(&b).unwrap();

    let mut other = env.other_workspace();
    assert_eq!(other.open(&a).unwrap().state, DataRootState::Open);
    assert_eq!(code(other.open(&b)), ErrorCode::DataRootInUse);

    // The locator now remembers b.
    let mut restarted = env.workspace();
    restarted.startup();
    assert_eq!(restarted.status().state, DataRootState::InUse);
    assert_eq!(restarted.status().display_path.as_deref(), b.to_str());
}

#[test]
fn errors_never_contain_the_path() {
    let env = Env::new();
    let root = env.base.join("secret-project-name");
    let mut ws = env.workspace_with(Arc::new(FakeVolumes {
        kind: VolumeKind::Network,
        free: GIB,
        total: GIB,
    }));
    let payload = ws.choose(&root).unwrap_err().payload();
    assert!(!payload.message.contains("secret-project-name"));
    assert!(payload.recovery.is_some());
}
