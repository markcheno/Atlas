//! Small filesystem helpers with crash-safety guarantees.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Replaces `target` with `bytes` so that a crash leaves either the old
/// content or the new content, never a partial file.
///
/// The bytes go to a uniquely named file in `staging_dir` (which must be on
/// the same volume as `target`), are flushed to disk, and are then renamed
/// over `target`. On Unix the parent directory is fsynced so the rename
/// itself is durable. `std::fs::rename` replaces an existing file on every
/// supported platform.
pub fn atomic_write(target: &Path, staging_dir: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::create_dir_all(staging_dir)?;
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "target has no file name"))?;
    let tmp = staging_dir.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        uuid::Uuid::new_v4().simple()
    ));

    let result = (|| {
        let mut file = File::create_new(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, target)?;
        if let Some(parent) = target.parent() {
            sync_dir(parent)?;
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Flushes a directory entry change (create, rename, delete) to disk.
#[cfg(unix)]
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Windows has no portable directory fsync; NTFS journals metadata.
#[cfg(not(unix))]
pub fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

/// The path itself if it exists, otherwise its closest existing ancestor.
pub fn nearest_existing_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|p| !p.as_os_str().is_empty() && p.exists())
        .map(Path::to_path_buf)
}

/// Checks writability by creating and removing a uniquely named empty file
/// in `dir`. This is the only reliable cross-platform test (permission bits
/// and ACLs disagree on Windows and network mounts).
pub fn probe_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(
        ".atlas-write-test-{}",
        uuid::Uuid::new_v4().simple()
    ));
    match File::create_new(&probe) {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Removes everything inside `dir` without removing `dir` itself.
pub fn clear_dir(dir: &Path) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_staging_files() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("tmp");
        let target = dir.path().join("settings.json");

        atomic_write(&target, &staging, b"first").unwrap();
        atomic_write(&target, &staging, b"second").unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"second");
        assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
    }

    #[test]
    fn atomic_write_failure_keeps_old_content() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("tmp");
        let target = dir.path().join("settings.json");
        atomic_write(&target, &staging, b"original").unwrap();

        // Renaming a file over a directory fails on every platform.
        let blocked = dir.path().join("blocked");
        fs::create_dir(&blocked).unwrap();
        fs::write(blocked.join("child"), b"x").unwrap();
        assert!(atomic_write(&blocked, &staging, b"new").is_err());

        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
    }

    #[test]
    fn nearest_ancestor_and_probe() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("a/b/c");
        assert_eq!(nearest_existing_ancestor(&deep).unwrap(), dir.path());
        assert!(probe_writable(dir.path()));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
