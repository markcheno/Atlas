//! Slice 0.5: durable inventory (phase0-spec §7; tests P0-05 to P0-13 and
//! P0-20 to P0-22). P0-08 with a real process kill is in
//! `inventory_crash.rs`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use atlas_lib::domain::dto::{
    InventoryOverview, JobState, PauseReason, PowerSource, ThrottleReason,
};
use atlas_lib::domain::traits::{CatalogRepository, SourceRootRecord};
use atlas_lib::error::ErrorCode;
use atlas_lib::infrastructure::native_path;
use atlas_lib::services::catalog::repository::SqliteCatalog;
use atlas_lib::services::inventory::{InventoryConfig, control};
use atlas_lib::services::sources;
use atlas_lib::services::workspace::Workspace;

mod common;
use common::{Env, GIB, InventoryHarness, code, fast_config, wait_until};

/// Bounds a failure, not a performance claim: shared CI runners are slow.
const LONG: Duration = Duration::from_secs(120);

struct Lib {
    env: Env,
    h: InventoryHarness,
    ws: Workspace,
}

impl Lib {
    fn new() -> Self {
        Self::with_config(fast_config())
    }

    fn with_config(config: InventoryConfig) -> Self {
        let env = Env::new();
        let h = InventoryHarness::new(config);
        let mut ws = env.workspace_with_inventory(h.deps.clone());
        ws.choose(&env.base.join("AtlasData")).unwrap();
        Self { env, h, ws }
    }

    fn reopen(&mut self) {
        self.ws = self.env.other_workspace(); // drops the open root first
        let mut ws = self.env.workspace_with_inventory(self.h.deps.clone());
        ws.startup();
        self.ws = ws;
    }

    fn dir(&self, rel: &str) -> PathBuf {
        let p = self.env.base.join(rel);
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// Adds a folder and schedules it, as the app's add command does.
    fn add(&self, path: &Path) -> (String, String) {
        let root = self.ws.require_open().unwrap();
        let added = sources::add(root, path).unwrap();
        let job = control::enqueue(root, &added.id).unwrap();
        (added.id, job)
    }

    fn overview(&self) -> InventoryOverview {
        control::overview(self.ws.require_open().unwrap()).unwrap()
    }

    fn state(&self, job: &str) -> Option<JobState> {
        let ov = self.overview();
        if let Some(a) = ov.active.as_ref().filter(|a| a.job_id == job) {
            return Some(a.state);
        }
        let id = job.to_owned();
        self.ws
            .require_open()
            .unwrap()
            .catalog()
            .call(move |c| {
                c.query_row("SELECT state FROM jobs WHERE id = ?1", [id], |r| {
                    r.get::<_, String>(0)
                })
                .ok()
            })
            .unwrap()
            .and_then(|s| atlas_lib::services::catalog::codec::from_db(&s))
    }

    fn wait_state(&self, job: &str, want: JobState) {
        assert!(
            wait_until(LONG, || self.state(job) == Some(want)),
            "job never reached {want:?}; now {:?}",
            self.state(job)
        );
    }

    /// (relative path → (id, availability, storage, updated_at)) for a root.
    fn files(&self, root_id: &str) -> BTreeMap<PathBuf, (String, String, String, String)> {
        let id = root_id.to_owned();
        self.ws
            .require_open()
            .unwrap()
            .catalog()
            .call(move |c| {
                let mut stmt = c
                    .prepare(
                        "SELECT rel_path, id, availability, storage, updated_at
                         FROM file_instances WHERE source_root_id = ?1",
                    )
                    .unwrap();
                stmt.query_map([id], |r| {
                    let rel: Vec<u8> = r.get(0)?;
                    Ok((
                        native_path::decode(&rel).unwrap(),
                        (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?),
                    ))
                })
                .unwrap()
                .collect::<Result<BTreeMap<_, _>, _>>()
                .unwrap()
            })
            .unwrap()
    }

    fn file_count(&self) -> usize {
        self.ws
            .require_open()
            .unwrap()
            .catalog()
            .call(|c| {
                c.query_row("SELECT COUNT(*) FROM file_instances", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap() as usize
            })
            .unwrap()
    }
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// A small tree with every naming quirk from phase0-spec §11.
fn populate(root: &Path) -> BTreeSet<PathBuf> {
    let files = [
        "Astronomy/Meteors/detection.pdf",
        "Astronomy/Meteors/NOISY VIDEO.PDF",
        "Engineering/harmonic drive – notes.epub",
        "Ünïcode/Café crème.pdf",
        "A/same.pdf",
        "B/same.pdf",
        "deep/a/b/c/d/e/f/deep.Epub",
        "punctuation/[draft] (v2) #1 & co.pdf",
    ];
    for f in files {
        write(&root.join(f), b"%PDF-1.7 stub");
    }
    write(&root.join("notes.txt"), b"text");
    write(&root.join("Astronomy/cover.png"), b"png");
    write(&root.join("archive.epub.zip"), b"zip");
    fs::create_dir_all(root.join("empty/also-empty")).unwrap();
    files.iter().map(PathBuf::from).collect()
}

type Snapshot = BTreeMap<PathBuf, (u64, Option<std::time::SystemTime>)>;

fn snapshot(root: &Path) -> Snapshot {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(&dir).unwrap() {
            let e = e.unwrap();
            let m = fs::symlink_metadata(e.path()).unwrap();
            out.insert(
                e.path().strip_prefix(root).unwrap().to_path_buf(),
                (m.len(), m.modified().ok()),
            );
            if m.is_dir() {
                stack.push(e.path());
            }
        }
    }
    out
}

fn many(root: &Path, n: usize) {
    for i in 0..n {
        write(
            &root.join(format!("d{:02}/book-{i:05}.pdf", i % 20)),
            b"%PDF",
        );
    }
}

/// P0-05: correct, unique file instances; nothing in the source changes.
#[test]
fn p0_05_catalogs_a_nested_tree_without_touching_it() {
    let lib = Lib::new();
    let books = lib.dir("Books");
    let expected = populate(&books);
    let before = snapshot(&books);

    let (root_id, job) = lib.add(&books);
    lib.wait_state(&job, JobState::Completed);

    let files = lib.files(&root_id);
    assert_eq!(files.keys().cloned().collect::<BTreeSet<_>>(), expected);
    assert!(
        files
            .values()
            .all(|(_, a, s, _)| a == "available" && s == "local")
    );
    assert_eq!(snapshot(&books), before);

    let ov = lib.overview();
    assert!(ov.active.is_none());
    assert_eq!(ov.cataloged_files as usize, expected.len());
    assert_eq!(ov.roots[0].file_count as usize, expected.len());
    assert_eq!(ov.roots[0].last_state, Some(JobState::Completed));
    assert!(ov.roots[0].last_scan_at.is_some());
    assert!(
        !lib.h.events.lock().unwrap().is_empty(),
        "progress events sent"
    );

    // Starting again while nothing changed is a fresh, equally exact rescan.
    let again = control::enqueue(lib.ws.require_open().unwrap(), &root_id).unwrap();
    assert_ne!(again, job);
    lib.wait_state(&again, JobState::Completed);
    assert_eq!(
        lib.files(&root_id),
        files,
        "unchanged rows keep IDs and timestamps"
    );
}

/// P0-06: an inner root of an overlapping pair is never scanned on its own.
#[test]
fn p0_06_overlapping_roots_are_not_double_scanned() {
    let lib = Lib::new();
    let outer = dunce::canonicalize(lib.dir("Library")).unwrap();
    let inner = outer.join("Math");
    write(&inner.join("calculus.pdf"), b"%PDF");
    write(&outer.join("history.pdf"), b"%PDF");

    let root = lib.ws.require_open().unwrap();
    let catalog = SqliteCatalog::new(root.catalog());
    for (id, path) in [("outer", &outer), ("inner", &inner)] {
        catalog
            .insert_source_root(SourceRootRecord {
                id: id.into(),
                workspace_id: root.manifest().root_id.clone(),
                path_bytes: native_path::encode(path),
                display_path: native_path::display(path),
                created_at: "t".into(),
                last_scan_at: None,
            })
            .unwrap();
    }
    assert_eq!(
        code(control::enqueue(root, "inner")),
        ErrorCode::SourceOverlaps
    );

    let job = control::enqueue(root, "outer").unwrap();
    lib.wait_state(&job, JobState::Completed);
    assert_eq!(lib.files("outer").len(), 2);
    assert!(lib.files("inner").is_empty());
    assert_eq!(lib.file_count(), 2);
}

/// P0-07: symlink loops and links leaving the root are skipped.
#[cfg(unix)]
#[test]
fn p0_07_symlinks_are_skipped_and_counted() {
    let lib = Lib::new();
    let books = lib.dir("Books");
    write(&books.join("inside.pdf"), b"%PDF");
    let outside = lib.dir("Outside");
    write(&outside.join("secret.pdf"), b"%PDF");
    std::os::unix::fs::symlink(&books, books.join("loop")).unwrap();
    std::os::unix::fs::symlink(&outside, books.join("escape")).unwrap();
    std::os::unix::fs::symlink(outside.join("secret.pdf"), books.join("linked.pdf")).unwrap();

    let (root_id, job) = lib.add(&books);
    lib.wait_state(&job, JobState::Completed);
    let files = lib.files(&root_id);
    assert_eq!(files.keys().collect::<Vec<_>>(), [Path::new("inside.pdf")]);
    let finished = lib.ws.require_open().unwrap().catalog().call({
        let job = job.clone();
        move |c| {
            c.query_row(
                "SELECT skipped_symlinks FROM jobs WHERE id = ?1",
                [job],
                |r| r.get::<_, u32>(0),
            )
            .unwrap()
        }
    });
    assert_eq!(finished.unwrap(), 3);
}

/// P0-09: a user pause survives a restart and is never lifted by itself.
#[test]
fn p0_09_user_pause_persists_across_restart() {
    let mut lib = Lib::new();
    let books = lib.dir("Books");
    many(&books, 400);
    lib.h.source.slow(Duration::from_millis(2));

    let (root_id, job) = lib.add(&books);
    assert!(wait_until(LONG, || lib.file_count() > 0));
    control::pause(lib.ws.require_open().unwrap(), &job).unwrap();
    lib.wait_state(&job, JobState::PausedUser);
    std::thread::sleep(Duration::from_millis(150));
    let frozen = lib.file_count();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(lib.file_count(), frozen, "no work while paused");
    assert!(frozen < 400);

    lib.reopen();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(lib.state(&job), Some(JobState::PausedUser));
    assert_eq!(lib.file_count(), frozen);

    lib.h.source.slow(Duration::ZERO);
    control::resume(lib.ws.require_open().unwrap(), &job).unwrap();
    lib.wait_state(&job, JobState::Completed);
    assert_eq!(lib.files(&root_id).len(), 400);
}

/// P0-10: low battery pauses bulk inventory; recovery resumes it.
#[test]
fn p0_10_low_battery_pauses_until_recovered() {
    let lib = Lib::new();
    let books = lib.dir("Books");
    many(&books, 50);
    lib.h.monitor.set(|s| {
        s.power = PowerSource::Battery;
        s.battery_present = Some(true);
        s.battery_percent = Some(18.0);
    });

    let (root_id, job) = lib.add(&books);
    lib.wait_state(&job, JobState::PausedPolicy);
    assert_eq!(
        lib.overview().active.unwrap().pause_reason,
        Some(PauseReason::BatteryLow)
    );
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(lib.file_count(), 0, "nothing written while paused");

    lib.h.monitor.set(|s| s.battery_percent = Some(30.0));
    lib.wait_state(&job, JobState::Completed);
    assert_eq!(lib.files(&root_id).len(), 50);
    let throttled_on_battery = lib.h.events.lock().unwrap().iter().any(|o| {
        o.active
            .as_ref()
            .is_some_and(|a| a.throttle_reason == Some(ThrottleReason::OnBattery))
    });
    assert!(throttled_on_battery);
}

/// P0-11: unknown power is conservative but still makes progress.
#[test]
fn p0_11_unknown_power_runs_throttled() {
    let lib = Lib::new();
    let books = lib.dir("Books");
    many(&books, 30);
    lib.h.monitor.set(|s| {
        s.power = PowerSource::Unknown;
        s.battery_present = None;
    });
    let (_, job) = lib.add(&books);
    lib.wait_state(&job, JobState::Completed);
    assert!(lib.h.events.lock().unwrap().iter().any(|o| {
        o.active
            .as_ref()
            .is_some_and(|a| a.throttle_reason == Some(ThrottleReason::PowerUnknown))
    }));
}

/// P0-12: low data-drive space pauses new write batches.
#[test]
fn p0_12_low_space_pauses_without_writing() {
    let lib = Lib::new();
    let books = lib.dir("Books");
    many(&books, 40);
    lib.h.monitor.set(|s| {
        s.data_volume_reachable = Some(true);
        s.data_volume_total_bytes = Some(100 * GIB);
        s.data_volume_free_bytes = Some(GIB);
    });

    let (root_id, job) = lib.add(&books);
    lib.wait_state(&job, JobState::PausedPolicy);
    assert_eq!(
        lib.overview().active.unwrap().pause_reason,
        Some(PauseReason::LowDiskSpace)
    );
    assert_eq!(lib.file_count(), 0);

    lib.h
        .monitor
        .set(|s| s.data_volume_free_bytes = Some(50 * GIB));
    lib.wait_state(&job, JobState::Completed);
    assert_eq!(lib.files(&root_id).len(), 40);
    let ok: String = lib
        .ws
        .require_open()
        .unwrap()
        .catalog()
        .call(|c| {
            c.query_row("PRAGMA integrity_check", [], |r| r.get(0))
                .unwrap()
        })
        .unwrap();
    assert_eq!(ok, "ok");
}

/// P0-13: an unplugged source pauses the scan and marks nothing missing.
#[test]
fn p0_13_source_disappearing_mid_scan_marks_nothing_missing() {
    let lib = Lib::new();
    let drive = lib.dir("Drive");
    let books = drive.join("Books");
    many(&books, 300);
    lib.h.source.slow(Duration::from_millis(2));

    let (root_id, job) = lib.add(&books);
    assert!(wait_until(LONG, || lib.file_count() > 20));
    // Unplug between two folder listings, then let the scan carry on.
    assert!(lib.h.source.hold_next_listing(LONG));
    fs::rename(&drive, lib.env.base.join("unplugged")).unwrap();
    lib.h.source.release();
    lib.wait_state(&job, JobState::PausedPolicy);
    assert_eq!(
        lib.overview().active.unwrap().pause_reason,
        Some(PauseReason::SourceRootMissing)
    );
    assert_eq!(lib.overview().missing_files, 0);

    fs::rename(lib.env.base.join("unplugged"), &drive).unwrap();
    lib.h.source.slow(Duration::ZERO);
    lib.wait_state(&job, JobState::Completed);
    let files = lib.files(&root_id);
    assert_eq!(files.len(), 300);
    assert!(files.values().all(|(_, a, _, _)| a == "available"));
}

/// P0-20: the data root inside a source is never cataloged.
#[test]
fn p0_20_data_root_inside_the_source_is_skipped() {
    let lib = Lib::new();
    let base = lib.env.base.clone();
    write(&base.join("Books/real.pdf"), b"%PDF");
    write(&base.join("AtlasData/exports/generated.pdf"), b"%PDF");

    let (root_id, job) = lib.add(&base);
    lib.wait_state(&job, JobState::Completed);
    let files = lib.files(&root_id);
    assert!(files.contains_key(Path::new("Books/real.pdf")));
    assert!(
        files.keys().all(|p| !p.starts_with("AtlasData")),
        "{files:?}"
    );
}

/// P0-21: closing the library stops the worker promptly; work resumes later.
#[test]
fn p0_21_closing_stops_the_worker_and_reopening_resumes() {
    let mut lib = Lib::new();
    let books = lib.dir("Books");
    many(&books, 400);
    lib.h.source.slow(Duration::from_millis(2));

    let (root_id, job) = lib.add(&books);
    assert!(wait_until(LONG, || lib.file_count() > 0));
    let started = Instant::now();
    lib.ws = lib.env.other_workspace(); // drops the open root and its worker
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );

    lib.h.source.slow(Duration::ZERO);
    let mut ws = lib.env.workspace_with_inventory(lib.h.deps.clone());
    ws.startup();
    lib.ws = ws;
    lib.wait_state(&job, JobState::Completed);
    assert_eq!(lib.files(&root_id).len(), 400);
}

/// P0-22: an unreadable folder is retried a bounded number of times, its
/// files are never marked missing, and deletions elsewhere still are.
#[test]
fn p0_22_unreadable_folder_is_bounded_and_protects_only_its_subtree() {
    let lib = Lib::new();
    let books = dunce::canonicalize(lib.dir("Books")).unwrap();
    write(&books.join("Locked/secret.pdf"), b"%PDF");
    write(&books.join("Open/fine.pdf"), b"%PDF");
    write(&books.join("Open/gone-later.pdf"), b"%PDF");

    let (root_id, first) = lib.add(&books);
    lib.wait_state(&first, JobState::Completed);
    assert_eq!(lib.files(&root_id).len(), 3);

    lib.h
        .source
        .fail_dir(&books.join("Locked"), ErrorKind::PermissionDenied);
    fs::remove_file(books.join("Open/gone-later.pdf")).unwrap();
    let second = control::enqueue(lib.ws.require_open().unwrap(), &root_id).unwrap();
    lib.wait_state(&second, JobState::Completed);

    assert_eq!(lib.h.source.calls_for(&books.join("Locked")), 1 + 3);
    let files = lib.files(&root_id);
    assert_eq!(
        files[Path::new("Locked/secret.pdf")].1,
        "available",
        "protected"
    );
    assert_eq!(files[Path::new("Open/gone-later.pdf")].1, "missing");
    assert_eq!(files[Path::new("Open/fine.pdf")].1, "available");
    assert_eq!(lib.overview().missing_files, 1);

    // If the root itself cannot be listed, nothing at all is marked missing.
    fs::remove_file(books.join("Open/fine.pdf")).unwrap();
    lib.h.source.fail_dir(&books, ErrorKind::PermissionDenied);
    let third = control::enqueue(lib.ws.require_open().unwrap(), &root_id).unwrap();
    lib.wait_state(&third, JobState::Completed);
    assert_eq!(
        lib.files(&root_id)[Path::new("Open/fine.pdf")].1,
        "available"
    );
}

/// §7.1 rescans: unchanged rows untouched, changes picked up, deletions
/// marked missing only after a clean scan.
#[test]
fn rescan_updates_changes_and_marks_deletions_missing() {
    let lib = Lib::new();
    let books = lib.dir("Books");
    write(&books.join("same.pdf"), b"%PDF");
    write(&books.join("edited.pdf"), b"%PDF");
    write(&books.join("deleted.epub"), b"PK");
    let (root_id, first) = lib.add(&books);
    lib.wait_state(&first, JobState::Completed);
    let before = lib.files(&root_id);

    std::thread::sleep(Duration::from_millis(1100)); // whole-second mtimes
    write(&books.join("edited.pdf"), b"%PDF with more bytes");
    fs::remove_file(books.join("deleted.epub")).unwrap();
    write(&books.join("new.pdf"), b"%PDF");
    let second = control::enqueue(lib.ws.require_open().unwrap(), &root_id).unwrap();
    lib.wait_state(&second, JobState::Completed);
    let after = lib.files(&root_id);

    let same = Path::new("same.pdf");
    assert_eq!(after[same], before[same]);
    let edited = Path::new("edited.pdf");
    assert_eq!(after[edited].0, before[edited].0, "same ID");
    assert_ne!(after[edited].3, before[edited].3, "updated_at changed");
    assert_eq!(after[Path::new("deleted.epub")].1, "missing");
    assert_eq!(after[Path::new("new.pdf")].1, "available");
    assert_eq!(lib.overview().missing_files, 1);
}

/// §7.1: a huge flat folder is processed in bounded batches.
#[test]
fn a_twenty_thousand_entry_folder_streams_in_batches() {
    let lib = Lib::with_config(InventoryConfig {
        batch_size: 100,
        ..fast_config()
    });
    let flat = lib.dir("Flat");
    for i in 0..20_000 {
        let ext = if i % 4 == 0 { "txt" } else { "pdf" };
        fs::write(flat.join(format!("f{i:05}.{ext}")), b"").unwrap();
    }
    let (root_id, job) = lib.add(&flat);
    lib.wait_state(&job, JobState::Completed);
    assert_eq!(lib.files(&root_id).len(), 15_000);
}

/// §7.3: cancel keeps results; terminal jobs refuse pause and resume.
#[test]
fn cancel_keeps_found_files_and_is_final() {
    let lib = Lib::new();
    let books = lib.dir("Books");
    many(&books, 300);
    lib.h.source.slow(Duration::from_millis(2));
    let (root_id, job) = lib.add(&books);
    assert!(wait_until(LONG, || lib.file_count() > 0));

    let root = lib.ws.require_open().unwrap();
    control::cancel(root, &job).unwrap();
    lib.wait_state(&job, JobState::Cancelled);
    std::thread::sleep(Duration::from_millis(100));
    let kept = lib.files(&root_id).len();
    assert!(kept > 0 && kept < 300);
    assert!(
        lib.files(&root_id)
            .values()
            .all(|(_, a, _, _)| a == "available")
    );
    assert_eq!(
        code(control::resume(root, &job)),
        ErrorCode::JobNotResumable
    );
    assert_eq!(code(control::pause(root, &job)), ErrorCode::JobNotPausable);
    assert_eq!(code(control::pause(root, "nope")), ErrorCode::JobNotFound);
}

/// Removing a folder mid-scan ends its job cleanly and drops its rows.
#[test]
fn removing_a_folder_mid_scan_ends_cleanly() {
    let lib = Lib::new();
    let books = lib.dir("Books");
    many(&books, 300);
    lib.h.source.slow(Duration::from_millis(2));
    let (root_id, job) = lib.add(&books);
    assert!(wait_until(LONG, || lib.file_count() > 0));

    sources::remove(lib.ws.require_open().unwrap(), &root_id).unwrap();
    assert!(wait_until(LONG, || lib.overview().active.is_none()));
    assert_eq!(lib.state(&job), None);
    assert_eq!(lib.file_count(), 0);
}

/// Elapsed time adds up across many sub-millisecond batches.
#[test]
fn elapsed_time_accumulates_across_batches() {
    let lib = Lib::new();
    let books = lib.dir("Books");
    many(&books, 200);
    lib.h.source.slow(Duration::from_millis(1));
    let started = Instant::now();
    let (_, job) = lib.add(&books);
    lib.wait_state(&job, JobState::Completed);
    let wall = started.elapsed().as_millis() as u64;
    let recorded: u64 = lib
        .ws
        .require_open()
        .unwrap()
        .catalog()
        .call(move |c| {
            c.query_row("SELECT elapsed_ms FROM jobs WHERE id = ?1", [job], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap() as u64
        })
        .unwrap();
    assert!(
        recorded >= 180,
        "recorded {recorded} ms for >=200 ms of listing"
    );
    assert!(recorded <= wall, "recorded {recorded} ms > wall {wall} ms");
}

/// Folders added before cataloging existed are cataloged on next open.
#[test]
fn never_scanned_folders_are_queued_on_open() {
    let mut lib = Lib::new();
    let books = lib.dir("Books");
    many(&books, 20);
    // Added without scheduling, as Slice 0.3 did.
    let added = sources::add(lib.ws.require_open().unwrap(), &books).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(lib.overview().active.is_none());

    lib.reopen();
    assert!(wait_until(LONG, || {
        lib.overview().roots[0].last_state == Some(JobState::Completed)
    }));
    assert_eq!(lib.files(&added.id).len(), 20);

    // Once scanned, reopening does not queue it again.
    lib.reopen();
    std::thread::sleep(Duration::from_millis(100));
    assert!(lib.overview().active.is_none());
}

/// Scheduling twice while a job is active returns the same job.
#[test]
fn enqueue_is_idempotent_while_active() {
    let lib = Lib::new();
    let books = lib.dir("Books");
    many(&books, 200);
    lib.h.source.slow(Duration::from_millis(2));
    let (root_id, job) = lib.add(&books);
    let root = lib.ws.require_open().unwrap();
    assert_eq!(control::enqueue(root, &root_id).unwrap(), job);
    control::pause(root, &job).unwrap();
    assert_eq!(
        control::enqueue(root, &root_id).unwrap(),
        job,
        "a paused job is kept"
    );
}

/// iCloud: a legacy placeholder is cataloged as the real file, cloud-only,
/// and keeps its identity when the file is downloaded.
#[cfg(target_os = "macos")]
#[test]
fn icloud_placeholders_are_cataloged_without_downloading() {
    let lib = Lib::new();
    let books = lib.dir("iCloud Books");
    let mut dict = plist::Dictionary::new();
    dict.insert("NSURLNameKey".into(), "Cosmos.pdf".into());
    dict.insert(
        "NSURLFileSizeKey".into(),
        plist::Value::Integer(48_213_004u64.into()),
    );
    plist::Value::Dictionary(dict)
        .to_file_binary(books.join(".Cosmos.pdf.icloud"))
        .unwrap();
    write(&books.join("Local.epub"), b"PK");

    let (root_id, first) = lib.add(&books);
    lib.wait_state(&first, JobState::Completed);
    let files = lib.files(&root_id);
    let cosmos = &files[Path::new("Cosmos.pdf")];
    assert_eq!(cosmos.2, "cloud_only");
    assert_eq!(files[Path::new("Local.epub")].2, "local");
    assert_eq!(lib.overview().cloud_only_files, 1);
    assert!(
        books.join(".Cosmos.pdf.icloud").exists(),
        "placeholder untouched"
    );

    // The user downloads it: the placeholder becomes the real file.
    fs::remove_file(books.join(".Cosmos.pdf.icloud")).unwrap();
    write(&books.join("Cosmos.pdf"), b"%PDF downloaded");
    let second = control::enqueue(lib.ws.require_open().unwrap(), &root_id).unwrap();
    lib.wait_state(&second, JobState::Completed);
    let after = &lib.files(&root_id)[Path::new("Cosmos.pdf")];
    assert_eq!(after.0, cosmos.0, "same catalog entry");
    assert_eq!(after.1, "available");
    assert_eq!(after.2, "local");
}
