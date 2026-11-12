//! Snapshots of the knowledge store (phase3-spec §6.3, ADR 0016).
//!
//! - **When:** the store changed since the last snapshot, and the newest
//!   is at least a day old (or there is none). Checked when a library
//!   opens and every hour while it stays open.
//! - **How:** `VACUUM INTO` a file in `backups/knowledge/staging/` on the
//!   writer thread, so the copy is exactly the store between two writes;
//!   `PRAGMA integrity_check` on the copy; an atomic rename to
//!   `backups/knowledge/knowledge-<UTC timestamp>.sqlite`. A copy that
//!   fails its check is deleted and reported.
//! - **Kept:** the newest [`KEEP`]. Older ones go only after a new one
//!   passed its check.
//! - **Low space** stops new snapshots and says so. Nothing is ever
//!   deleted to make room.

use crate::services::diagnostics::Event;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time};

use super::meta;
use super::store::Inner;
use crate::domain::dto::{KnowledgeState, SnapshotOutcome};
use crate::infrastructure::fsutil::sync_dir;
use crate::services::system::low_space_threshold;

/// Snapshots kept.
pub const KEEP: usize = 14;
/// At most one snapshot a day.
pub const EVERY: time::Duration = time::Duration::hours(24);

const PREFIX: &str = "knowledge-";
const SUFFIX: &str = ".sqlite";

/// A snapshot file in `backups/knowledge/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub path: PathBuf,
    pub at: OffsetDateTime,
    pub bytes: u64,
}

/// `knowledge-20261111T093000Z.sqlite`: sortable, and valid on every OS.
pub fn file_name(at: OffsetDateTime, n: u32) -> String {
    let t = at.to_offset(time::UtcOffset::UTC);
    let extra = if n == 0 {
        String::new()
    } else {
        format!("-{n}")
    };
    format!(
        "{PREFIX}{:04}{:02}{:02}T{:02}{:02}{:02}Z{extra}{SUFFIX}",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    )
}

/// The time in a snapshot's file name; `None` for any other file.
pub fn parse_name(name: &str) -> Option<OffsetDateTime> {
    let stamp = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let (stamp, extra) = stamp.split_at_checked(16)?;
    if !(extra.is_empty() || extra.strip_prefix('-')?.parse::<u32>().is_ok()) {
        return None;
    }
    let b = stamp.as_bytes();
    if b[8] != b'T' || b[15] != b'Z' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| stamp.get(r)?.parse::<u32>().ok();
    let date = Date::from_calendar_date(
        num(0..4)? as i32,
        Month::try_from(num(4..6)? as u8).ok()?,
        num(6..8)? as u8,
    )
    .ok()?;
    let time = Time::from_hms(num(9..11)? as u8, num(11..13)? as u8, num(13..15)? as u8).ok()?;
    Some(PrimitiveDateTime::new(date, time).assume_utc())
}

/// The snapshots in `dir`, oldest first.
pub fn list(dir: &Path) -> Vec<Snapshot> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, Snapshot)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let at = parse_name(&name)?;
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            Some((
                name,
                Snapshot {
                    path: e.path(),
                    at,
                    bytes: meta.len(),
                },
            ))
        })
        .collect();
    out.sort_by(|a, b| (a.1.at, &a.0).cmp(&(b.1.at, &b.0)));
    out.into_iter().map(|(_, s)| s).collect()
}

/// Removes whatever an interrupted snapshot left in `staging`.
pub fn clean_staging(staging: &Path) {
    if let Ok(entries) = fs::read_dir(staging) {
        for e in entries.flatten() {
            if e.file_type().is_ok_and(|t| t.is_file()) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

fn record(inner: &Inner, outcome: SnapshotOutcome) -> SnapshotOutcome {
    *inner.last.lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
    outcome
}

/// The store's change count, and the count its newest snapshot captured.
fn changes(conn: &Connection) -> rusqlite::Result<(i64, Option<i64>)> {
    Ok((
        meta::get_int(conn, "changes")?.unwrap_or(0),
        meta::get_int(conn, "last_snapshot_change")?,
    ))
}

/// Takes a snapshot if one is due. `None` when the store isn't ready.
pub fn if_due(inner: &Inner) -> Option<SnapshotOutcome> {
    if inner.state != KnowledgeState::Ready {
        return None;
    }
    let _one = inner.snapshotting.lock().unwrap_or_else(|p| p.into_inner());
    let writer = inner.writer("snapshot").ok()?;
    let Ok(Ok((now, last))) = writer.call(|c| changes(c)) else {
        return Some(record(inner, SnapshotOutcome::Failed));
    };
    if last == Some(now) || (last.is_none() && now == 0) {
        return Some(record(inner, SnapshotOutcome::NotChanged));
    }
    let snaps = list(&inner.snapshots_dir());
    if snaps
        .last()
        .is_some_and(|s| inner.clock.now() - s.at < EVERY)
    {
        return Some(record(inner, SnapshotOutcome::NotDue));
    }
    Some(take_locked(inner, &|_| {}))
}

/// Takes a snapshot now, whatever the rule says (the kill test's child,
/// and [`if_due`]). `None` when the store isn't ready.
pub fn take(inner: &Inner) -> Option<SnapshotOutcome> {
    if inner.state != KnowledgeState::Ready {
        return None;
    }
    let _one = inner.snapshotting.lock().unwrap_or_else(|p| p.into_inner());
    Some(take_locked(inner, &|_| {}))
}

/// Whether the drive holding the library is nearly full.
fn low_space(inner: &Inner) -> bool {
    inner.volumes.inspect(&inner.root).is_ok_and(|v| {
        matches!((v.free_bytes, v.total_bytes), (Some(free), Some(total)) if free < low_space_threshold(total))
    })
}

/// `before_check` runs on the staged copy before it is checked: tests use
/// it to damage the copy.
fn take_locked(inner: &Inner, before_check: &dyn Fn(&Path)) -> SnapshotOutcome {
    let started = std::time::Instant::now();
    let outcome = take_timed(inner, before_check);
    if let Some(d) = inner.diagnostics.get() {
        d.record(Event::Snapshot {
            outcome,
            ms: started.elapsed().as_millis() as u64,
        });
    }
    outcome
}

fn take_timed(inner: &Inner, before_check: &dyn Fn(&Path)) -> SnapshotOutcome {
    if low_space(inner) {
        return record(inner, SnapshotOutcome::LowSpace);
    }
    let staging = inner.staging_dir();
    let dir = inner.snapshots_dir();
    if fs::create_dir_all(&staging).is_err() {
        return record(inner, SnapshotOutcome::Failed);
    }
    let at = inner.clock.now();
    let staged = staging.join(file_name(at, 0));
    remove_with_sides(&staged);
    let Some(staged_text) = staged.to_str().map(str::to_owned) else {
        return record(inner, SnapshotOutcome::Failed);
    };
    let Ok(writer) = inner.writer("snapshot") else {
        return record(inner, SnapshotOutcome::Failed);
    };
    // On the writer thread, so the copy and the count it records are of
    // the same moment.
    let copied = writer.call(move |c| -> rusqlite::Result<i64> {
        let (count, _) = changes(c)?;
        c.execute("VACUUM INTO ?1", [&staged_text])?;
        Ok(count)
    });
    let Ok(Ok(count)) = copied else {
        remove_with_sides(&staged);
        return record(inner, SnapshotOutcome::Failed);
    };
    before_check(&staged);
    if !verify(&staged) {
        remove_with_sides(&staged);
        return record(inner, SnapshotOutcome::Failed);
    }
    remove_sides(&staged);
    let mut n = 0;
    let target = loop {
        let t = dir.join(file_name(at, n));
        if !t.exists() {
            break t;
        }
        n += 1;
    };
    if fs::rename(&staged, &target).is_err() {
        remove_with_sides(&staged);
        return record(inner, SnapshotOutcome::Failed);
    }
    let _ = sync_dir(&dir);
    let stamp = crate::infrastructure::clock::rfc3339(at);
    let _ = writer.call(move |c| {
        meta::set(c, "last_snapshot_at", &stamp)?;
        meta::set(c, "last_snapshot_change", &count.to_string())
    });
    prune(&dir, KEEP);
    record(inner, SnapshotOutcome::Taken)
}

/// Whether the copy at `path` is a whole, sound store.
pub fn verify(path: &Path) -> bool {
    let checked = (|| {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let ok: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        let version = super::migrations::current_version(&conn)?;
        Ok::<_, rusqlite::Error>(ok == "ok" && version >= 1)
    })();
    matches!(checked, Ok(true))
}

/// Removes the oldest snapshots beyond `keep`.
pub fn prune(dir: &Path, keep: usize) {
    let snaps = list(dir);
    let extra = snaps.len().saturating_sub(keep);
    for s in &snaps[..extra] {
        let _ = fs::remove_file(&s.path);
    }
}

fn sides(path: &Path) -> [PathBuf; 2] {
    ["-wal", "-shm"].map(|s| PathBuf::from(format!("{}{s}", path.display())))
}

fn remove_sides(path: &Path) {
    for side in sides(path) {
        let _ = fs::remove_file(side);
    }
}

fn remove_with_sides(path: &Path) {
    let _ = fs::remove_file(path);
    remove_sides(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::domain::dto::VolumeKind;
    use crate::domain::traits::{VolumeInfo, VolumeInspector};
    use crate::error::AtlasResult;
    use crate::infrastructure::clock::TestClock;
    use crate::services::knowledge::KnowledgeStore;
    use time::macros::datetime;

    struct Roomy;
    impl VolumeInspector for Roomy {
        fn inspect(&self, _: &Path) -> AtlasResult<VolumeInfo> {
            Ok(VolumeInfo {
                free_bytes: Some(500 << 30),
                total_bytes: Some(1000 << 30),
                fs_type: None,
                kind: VolumeKind::Local,
            })
        }
    }

    #[test]
    fn names_sort_by_time_and_only_snapshots_parse() {
        let at = datetime!(2026-11-11 09:30:05 UTC);
        assert_eq!(file_name(at, 0), "knowledge-20261111T093005Z.sqlite");
        assert_eq!(file_name(at, 2), "knowledge-20261111T093005Z-2.sqlite");
        assert_eq!(parse_name(&file_name(at, 0)), Some(at));
        assert_eq!(parse_name(&file_name(at, 2)), Some(at));
        for other in [
            "knowledge.sqlite",
            "knowledge-20261111T093005Z.sqlite-wal",
            "knowledge-2026111XT093005Z.sqlite",
            "knowledge-20261311T093005Z.sqlite",
            "knowledge-20261111T093005Z-x.sqlite",
            "notes.txt",
        ] {
            assert_eq!(parse_name(other), None, "{other}");
        }
    }

    #[test]
    fn a_copy_that_fails_its_check_is_removed_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(TestClock::at(datetime!(2026-11-11 09:00 UTC)));
        let store = KnowledgeStore::open(dir.path(), clock, Arc::new(Roomy), Arc::default());
        store
            .call(|c| {
                c.execute(
                    "INSERT INTO collections (id, kind, name, created_at, updated_at)
                     VALUES ('c', 'interest', 'Birds', 'a', 'a')",
                    [],
                )
            })
            .unwrap()
            .unwrap();
        let outcome = take_locked(store.inner(), &|staged| {
            fs::write(staged, b"not a database at all").unwrap();
        });
        assert_eq!(outcome, SnapshotOutcome::Failed);
        assert!(list(&store.inner().snapshots_dir()).is_empty());
        let staged = fs::read_dir(store.inner().staging_dir()).unwrap().count();
        assert_eq!(staged, 0, "the bad copy is gone");
        assert_eq!(
            store.status().snapshots.last_outcome,
            Some(SnapshotOutcome::Failed)
        );
        // The store itself is fine, and the next check takes one.
        assert_eq!(store.snapshot_if_due(), Some(SnapshotOutcome::Taken));
    }

    #[test]
    fn pruning_keeps_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        let base = datetime!(2026-01-01 00:00 UTC);
        for d in 0..20 {
            let name = file_name(base + time::Duration::days(d), 0);
            fs::write(dir.path().join(name), b"x").unwrap();
        }
        fs::write(dir.path().join("README.txt"), b"mine").unwrap();
        prune(dir.path(), KEEP);
        let left = list(dir.path());
        assert_eq!(left.len(), KEEP);
        assert_eq!(left[0].at, base + time::Duration::days(6));
        assert!(dir.path().join("README.txt").exists(), "not a snapshot");
    }
}
