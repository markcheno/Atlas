//! Slice 4.5: a rescan writes only what changed (phase4-spec §7.4, P4-11;
//! ADR 0022). The detection itself is Phase 0's and Phase 1's, so the
//! second half repeats their cases on one folder: a new, a changed, a
//! moved, a deleted and a cloud-only file, a folder deleted, and a folder
//! that can't be read.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use atlas_lib::domain::dto::ScanOrigin;
use atlas_lib::infrastructure::native_path;
use atlas_lib::services::inventory::control;
use atlas_lib::services::sources;
use atlas_lib::services::workspace::Workspace;

mod common;
use common::{Env, InventoryHarness, fast_config, wait_until};

const LONG: Duration = Duration::from_secs(120);

struct Lib {
    env: Env,
    h: InventoryHarness,
    ws: Workspace,
}

impl Lib {
    fn new() -> Self {
        let env = Env::new();
        let h = InventoryHarness::new(fast_config());
        let mut ws = env.workspace_with_inventory(h.deps.clone());
        ws.choose(&env.base.join("AtlasData")).unwrap();
        Self { env, h, ws }
    }

    fn add(&self, path: &Path) -> (String, String) {
        let root = self.ws.require_open().unwrap();
        let added = sources::add(root, path).unwrap();
        let job = control::enqueue(root, &added.id).unwrap();
        (added.id, job)
    }

    fn sql<T: rusqlite::types::FromSql + Send + 'static>(&self, q: &str) -> T {
        let q = q.to_owned();
        self.ws
            .require_open()
            .unwrap()
            .catalog()
            .call(move |c| c.query_row(&q, [], |r| r.get(0)).unwrap())
            .unwrap()
    }

    fn exec(&self, q: &str) {
        let q = q.to_owned();
        self.ws
            .require_open()
            .unwrap()
            .catalog()
            .call(move |c| c.execute_batch(&q).unwrap())
            .unwrap();
    }

    fn wait_completed(&self, job: &str) {
        let q = format!("SELECT state FROM jobs WHERE id = '{job}'");
        assert!(
            wait_until(LONG, || self.sql::<String>(&q) == "completed"),
            "the scan never completed: {}",
            self.sql::<String>(&q)
        );
    }

    fn rescan(&self, root_id: &str) -> String {
        let job = control::rescan(self.ws.require_open().unwrap(), root_id).unwrap();
        self.wait_completed(&job);
        job
    }

    /// (new, changed, missing, discovered) as the scan counted them.
    fn counts(&self, job: &str) -> (u32, u32, u32, u32) {
        let scans =
            control::scans(self.ws.require_open().unwrap(), &self.root_of(job), 50).unwrap();
        let s = scans.into_iter().find(|s| s.job_id == job).unwrap();
        (
            s.new_files,
            s.changed_files,
            s.missing_files,
            s.discovered_files,
        )
    }

    fn root_of(&self, job: &str) -> String {
        self.sql(&format!(
            "SELECT source_root_id FROM jobs WHERE id = '{job}'"
        ))
    }

    fn availability(&self, rel: &str) -> String {
        let rel_bytes = native_path::encode(&rel.split('/').fold(PathBuf::new(), |p, s| p.join(s)));
        let hex: String = rel_bytes.iter().map(|b| format!("{b:02x}")).collect();
        self.sql(&format!(
            "SELECT availability FROM file_instances WHERE rel_path = x'{hex}'"
        ))
    }

    /// Writes to `file_instances` since the counter was set up.
    fn writes(&self) -> i64 {
        self.sql("SELECT COUNT(*) FROM test_writes")
    }
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
fn p4_11_an_unchanged_rescan_writes_nothing_and_changes_are_found_as_before() {
    let lib = Lib::new();
    let books = lib.env.base.join("Books");
    for rel in [
        "a.pdf",
        "b.epub",
        "sub/c.pdf",
        "sub/deep/d.pdf",
        "keep/e.pdf",
        "keep/f.pdf",
    ] {
        write(&books.join(rel), b"%PDF-1.7 first");
    }
    let (root_id, first) = lib.add(&books);
    lib.wait_completed(&first);
    assert_eq!(lib.counts(&first), (6, 0, 0, 6));

    // Count every write to the files' rows from here on.
    lib.exec(
        "CREATE TABLE test_writes (op TEXT);
         CREATE TRIGGER test_write_update AFTER UPDATE ON file_instances
             BEGIN INSERT INTO test_writes VALUES ('update'); END;
         CREATE TRIGGER test_write_insert AFTER INSERT ON file_instances
             BEGIN INSERT INTO test_writes VALUES ('insert'); END;
         CREATE TRIGGER test_write_delete AFTER DELETE ON file_instances
             BEGIN INSERT INTO test_writes VALUES ('delete'); END;",
    );

    // Nothing changed: no row is written, nothing is missing.
    let same = lib.rescan(&root_id);
    assert_eq!(lib.writes(), 0, "an unchanged rescan wrote a file's row");
    assert_eq!(lib.counts(&same), (0, 0, 0, 6));
    let origin: String = lib.sql(&format!("SELECT origin FROM jobs WHERE id = '{same}'"));
    assert_eq!(origin, "user");

    // Each kind of change, once, in one rescan.
    std::thread::sleep(Duration::from_millis(1100)); // whole-second mtimes
    write(&books.join("a.pdf"), b"%PDF-1.7 edited, longer"); // changed
    fs::remove_file(books.join("b.epub")).unwrap(); // deleted
    write(&books.join("sub/new.pdf"), b"%PDF-1.7 new"); // new
    fs::create_dir_all(books.join("moved")).unwrap();
    fs::rename(books.join("keep/e.pdf"), books.join("moved/e.pdf")).unwrap(); // moved
    fs::remove_dir_all(books.join("sub/deep")).unwrap(); // a folder deleted
    let c = dunce::canonicalize(books.join("sub/c.pdf")).unwrap();
    lib.h.source.mark_cloud_only(&c); // offloaded to the cloud
    let changes = lib.rescan(&root_id);
    // New: sub/new.pdf, moved/e.pdf. Changed: a.pdf, sub/c.pdf (now in the
    // cloud). Missing: b.epub, keep/e.pdf, sub/deep/d.pdf.
    assert_eq!(lib.counts(&changes), (2, 2, 3, 5));
    for (rel, want) in [
        ("a.pdf", "available"),
        ("b.epub", "missing"),
        ("sub/new.pdf", "available"),
        ("moved/e.pdf", "available"),
        ("keep/e.pdf", "missing"),
        ("keep/f.pdf", "available"),
        ("sub/deep/d.pdf", "missing"),
        ("sub/c.pdf", "available"),
    ] {
        assert_eq!(lib.availability(rel), want, "{rel}");
    }
    let storage: String =
        lib.sql("SELECT storage FROM file_instances WHERE display_name = 'c.pdf'");
    assert_eq!(storage, "cloud_only");
    // a.pdf is read again, the moved file links by content when read.
    let a_state: String =
        lib.sql("SELECT extract_state FROM file_instances WHERE display_name = 'a.pdf'");
    assert_eq!(a_state, "pending");

    // A file back from the missing is written again (it is available).
    write(&books.join("b.epub"), b"PK back");
    let back = lib.rescan(&root_id);
    assert_eq!(lib.counts(&back).0 + lib.counts(&back).1, 1);
    assert_eq!(lib.availability("b.epub"), "available");
}

#[test]
fn a_folder_that_cannot_be_read_keeps_its_files_and_is_recorded() {
    let lib = Lib::new();
    let books = lib.env.base.join("Books");
    for rel in ["a.pdf", "locked/b.pdf", "locked/c.pdf"] {
        write(&books.join(rel), b"%PDF");
    }
    let (root_id, first) = lib.add(&books);
    lib.wait_completed(&first);

    let locked = dunce::canonicalize(books.join("locked")).unwrap();
    lib.h.source.fail_dir(&locked, ErrorKind::PermissionDenied);
    let job = lib.rescan(&root_id);
    assert_eq!(lib.availability("locked/b.pdf"), "available");
    assert_eq!(lib.availability("locked/c.pdf"), "available");
    let scans = control::scans(lib.ws.require_open().unwrap(), &root_id, 10).unwrap();
    assert_eq!(scans[0].job_id, job);
    assert_eq!(scans[0].problems, 1);
    assert_eq!(scans[0].missing_files, 0);
    let problems = control::problems(lib.ws.require_open().unwrap(), &job).unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].code, "permission_denied");
    assert!(problems[0].display_path.ends_with("locked"), "{problems:?}");
    // The earlier scan had none; the history is newest first.
    assert_eq!(scans[1].job_id, first);
    assert_eq!(scans[1].problems, 0);
    assert_eq!(scans[1].origin, Some(ScanOrigin::User));
}

#[test]
fn rescan_resumes_a_scan_the_user_paused_and_check_all_skips_what_is_away() {
    let lib = Lib::new();
    let books = lib.env.base.join("Books");
    let drive = lib.env.base.join("Drive");
    for i in 0..300 {
        write(&books.join(format!("{i:03}.pdf")), b"%PDF");
    }
    write(&drive.join("Away/x.pdf"), b"%PDF");
    lib.h.source.slow(Duration::from_millis(2));
    let (root_id, job) = lib.add(&books);
    let root = lib.ws.require_open().unwrap();
    control::pause(root, &job).unwrap();
    let q = format!("SELECT state FROM jobs WHERE id = '{job}'");
    assert!(wait_until(LONG, || lib.sql::<String>(&q) == "paused_user"));
    // Rescan on a paused folder resumes its scan instead of doing nothing.
    assert_eq!(control::rescan(root, &root_id).unwrap(), job);
    assert!(wait_until(LONG, || lib.sql::<String>(&q) != "paused_user"));
    lib.h.source.slow(Duration::ZERO);
    lib.wait_completed(&job);

    let (away_id, away_job) = lib.add(&drive.join("Away"));
    lib.wait_completed(&away_job);
    fs::rename(&drive, lib.env.base.join("Unplugged")).unwrap();
    // Two folders, one of them away: one check is scheduled.
    assert_eq!(control::check_all(root).unwrap(), 1);
    let scheduled: i64 = lib.sql(&format!(
        "SELECT COUNT(*) FROM jobs WHERE source_root_id = '{away_id}' AND kind = 'scan'
           AND state = 'queued'"
    ));
    assert_eq!(scheduled, 0, "no check for a folder that is away");
}
