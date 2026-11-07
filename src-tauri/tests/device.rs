//! Slice 0.4: device check assembly (phase0-spec §5.4, §7.2, §8).

use std::fs;
use std::sync::Arc;

use atlas_lib::domain::dto::{AssessmentNote, PauseReason, PolicyMode, VolumeKind};
use atlas_lib::infrastructure::system_probe::OsResourceMonitor;
use atlas_lib::services::scheduler::policy::ResourcePolicy;
use atlas_lib::services::system::{device_check, low_space_threshold};

mod common;
use common::{FakeVolumes, GIB, local};

#[test]
fn low_space_on_the_data_drive_pauses_work_and_is_noted() {
    let dir = tempfile::tempdir().unwrap();
    let monitor = OsResourceMonitor::new(Arc::new(FakeVolumes {
        kind: VolumeKind::Local,
        free: GIB,
        total: 500 * GIB,
    }));
    let check = device_check(&monitor, &mut ResourcePolicy::new(), Some(dir.path()));
    assert_eq!(check.snapshot.data_volume_free_bytes, Some(GIB));
    assert_eq!(
        check.snapshot.required_free_bytes,
        Some(low_space_threshold(500 * GIB))
    );
    assert_eq!(check.policy.mode, PolicyMode::Paused);
    assert_eq!(check.policy.pause_reason, Some(PauseReason::LowDiskSpace));
    assert!(check.assessment.notes.contains(&AssessmentNote::LowSpace));
}

#[test]
fn without_an_open_library_the_drive_is_not_considered() {
    let monitor = OsResourceMonitor::new(local());
    let check = device_check(&monitor, &mut ResourcePolicy::new(), None);
    assert_eq!(check.snapshot.data_volume_reachable, None);
    assert_ne!(
        check.policy.pause_reason,
        Some(PauseReason::DataRootMissing)
    );
    assert_ne!(check.policy.pause_reason, Some(PauseReason::LowDiskSpace));
}

#[test]
fn an_unplugged_data_drive_pauses_work() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Atlas Library");
    fs::create_dir_all(&root).unwrap();
    let monitor = OsResourceMonitor::new(local());
    let mut policy = ResourcePolicy::new();
    assert_ne!(
        device_check(&monitor, &mut policy, Some(&root))
            .policy
            .pause_reason,
        Some(PauseReason::DataRootMissing)
    );
    fs::remove_dir(&root).unwrap();
    let check = device_check(&monitor, &mut policy, Some(&root));
    assert_eq!(check.snapshot.data_volume_reachable, Some(false));
    assert_eq!(
        check.policy.pause_reason,
        Some(PauseReason::DataRootMissing)
    );
}
