//! Exclusive per-root lock so only one Atlas process writes a data root.
//!
//! Uses the OS file lock (`flock` on Unix, `LockFileEx` on Windows) via
//! `std::fs::File::try_lock`. The OS releases it when the process exits or
//! crashes, so a leftover `atlas.lock` file never blocks anyone: the lock,
//! not the file's existence, is the signal. The file's contents (process ID
//! and version) are informational only.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::Path;

use super::layout::LOCK;
use crate::error::{AtlasError, AtlasResult};

#[derive(Debug)]
pub struct RootLock {
    _file: File,
}

pub fn acquire(root: &Path) -> AtlasResult<RootLock> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(LOCK))
        .map_err(AtlasError::io("open the library lock"))?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(AtlasError::DataRootInUse),
        Err(TryLockError::Error(err)) => return Err(AtlasError::io("lock the library")(err)),
    }
    // Best effort; failure to annotate does not affect locking.
    let _ = file.set_len(0).and_then(|()| {
        writeln!(
            file,
            "pid={} version={}",
            std::process::id(),
            env!("CARGO_PKG_VERSION")
        )
    });
    Ok(RootLock { _file: file })
}
