//! Live availability checks for source roots.
//!
//! Each root is probed on its own short-lived thread and the caller waits
//! at most a fixed budget for all of them, so one hung network mount
//! cannot freeze the UI (P0-22). A probe that misses the deadline reports
//! `Unavailable`; its thread finishes or stays blocked in the OS without
//! holding any Atlas lock.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::domain::dto::SourceRootStatus;

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Lists nothing beyond the first directory read, and writes nothing.
pub fn probe(path: &Path) -> SourceRootStatus {
    match fs::read_dir(path) {
        Ok(_) => SourceRootStatus::Available,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => SourceRootStatus::NoAccess,
        Err(_) => SourceRootStatus::Unavailable,
    }
}

/// Probes every path concurrently. `None` entries (paths that could not be
/// decoded on this platform) are reported unavailable.
pub fn probe_all(paths: &[Option<PathBuf>]) -> Vec<SourceRootStatus> {
    probe_all_with(paths, PROBE_TIMEOUT, probe)
}

pub fn probe_all_with(
    paths: &[Option<PathBuf>],
    timeout: Duration,
    probe_fn: fn(&Path) -> SourceRootStatus,
) -> Vec<SourceRootStatus> {
    let mut statuses = vec![SourceRootStatus::Unavailable; paths.len()];
    let (tx, rx) = mpsc::channel();
    let mut pending = 0usize;

    for (index, path) in paths.iter().enumerate() {
        let Some(path) = path.clone() else { continue };
        let tx = tx.clone();
        let spawned = thread::Builder::new()
            .name("atlas-source-probe".into())
            .spawn(move || {
                let _ = tx.send((index, probe_fn(&path)));
            });
        if spawned.is_ok() {
            pending += 1;
        }
    }
    drop(tx);

    let deadline = Instant::now() + timeout;
    while pending > 0 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok((index, status)) => {
                statuses[index] = status;
                pending -= 1;
            }
            Err(_) => break,
        }
    }
    statuses
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_available_missing_and_undecodable() {
        let dir = tempfile::tempdir().unwrap();
        let statuses = probe_all(&[
            Some(dir.path().to_path_buf()),
            Some(dir.path().join("gone")),
            None,
        ]);
        assert_eq!(
            statuses,
            [
                SourceRootStatus::Available,
                SourceRootStatus::Unavailable,
                SourceRootStatus::Unavailable
            ]
        );
    }

    #[test]
    fn a_hung_probe_does_not_block_the_caller() {
        fn hangs(_: &Path) -> SourceRootStatus {
            thread::sleep(Duration::from_secs(10));
            SourceRootStatus::Available
        }
        let started = Instant::now();
        let statuses = probe_all_with(
            &[Some(PathBuf::from("/"))],
            Duration::from_millis(200),
            hangs,
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(statuses, [SourceRootStatus::Unavailable]);
    }
}
