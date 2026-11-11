//! Slice 3.2: the knowledge store (phase3-spec §6, ADR 0016).
//!
//! - P3-04: created on first open; migrations; a store from a newer Atlas,
//!   or one Atlas can't read, never blocks the library and is never
//!   altered.
//! - P3-06 (the rule): a snapshot when the store changed, at most daily;
//!   the fifteenth prunes the oldest. The kill case is in
//!   `knowledge_crash.rs`.
//! - P3-08: anchors resolve against the catalog as it is now, and never
//!   write.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use atlas_lib::domain::dto::{AnchorState, DataRootState, KnowledgeState, SnapshotOutcome};
use atlas_lib::error::ErrorCode;
use atlas_lib::infrastructure::clock::{Clock, TestClock};
use atlas_lib::services::extraction::db;
use atlas_lib::services::knowledge::anchor::{self, Place};
use atlas_lib::services::knowledge::migrations::SUPPORTED_KNOWLEDGE_VERSION;
use atlas_lib::services::knowledge::{SNAPSHOT_STAGING_DIR, SNAPSHOTS_DIR, snapshot};
use atlas_lib::services::workspace::layout::KNOWLEDGE;
use atlas_lib::services::workspace::{OpenRoot, Workspace};
use atlas_lib::services::{inventory, sources};
use rusqlite::Connection;
use time::macros::datetime;

mod common;
use common::{Env, ExtractionHarness, code, fast_extraction, wait_until, worker};

const LONG: Duration = Duration::from_secs(180);

fn knowledge_of(ws: &Workspace) -> &atlas_lib::services::knowledge::KnowledgeStore {
    ws.require_open().unwrap().knowledge()
}

fn add_collection(ws: &Workspace, id: &str) {
    let id = id.to_owned();
    knowledge_of(ws)
        .call(move |c| {
            c.execute(
                "INSERT INTO collections (id, kind, name, created_at, updated_at)
                 VALUES (?1, 'interest', 'Birds', 't', 't')",
                [id],
            )
        })
        .unwrap()
        .unwrap();
}

/// Closes cleanly, with the log folded into the file, so a test can
/// change the file and compare its bytes.
fn checkpoint(path: &Path) {
    let c = Connection::open(path).unwrap();
    c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
}

// ----------------------------------------------------------------- P3-04

#[test]
fn p3_04_created_on_first_open_and_reopened_as_it_was() {
    let env = Env::new();
    let root = env.base.join("lib");
    {
        let mut ws = env.workspace();
        ws.choose(&root).unwrap();
        let store = knowledge_of(&ws);
        assert_eq!(store.state(), KnowledgeState::Ready);
        assert!(root.join(KNOWLEDGE).is_file());
        assert!(root.join(SNAPSHOTS_DIR).is_dir() && root.join(SNAPSHOT_STAGING_DIR).is_dir());
        add_collection(&ws, "c1");
        let status = store.status();
        assert_eq!(status.collections, 1);
        assert!(status.bytes > 0);
    }
    let c = Connection::open(root.join(KNOWLEDGE)).unwrap();
    let version: u32 = c
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, SUPPORTED_KNOWLEDGE_VERSION);
    let mode: String = c
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    drop(c);

    let mut ws = env.workspace();
    ws.startup();
    assert_eq!(ws.status().state, DataRootState::Open);
    let store = knowledge_of(&ws);
    assert_eq!(store.state(), KnowledgeState::Ready);
    assert_eq!(store.status().collections, 1, "the row survives a reopen");
}

/// A store at `version`, closed cleanly; returns its bytes.
fn set_version(root: &Path, version: u32) -> Vec<u8> {
    let path = root.join(KNOWLEDGE);
    {
        let c = Connection::open(&path).unwrap();
        c.pragma_update(None, "user_version", version).unwrap();
    }
    checkpoint(&path);
    fs::read(&path).unwrap()
}

#[test]
fn p3_04_a_newer_store_is_left_alone_and_the_library_still_opens() {
    let env = Env::new();
    let root = env.base.join("lib");
    {
        let mut ws = env.workspace();
        ws.choose(&root).unwrap();
        add_collection(&ws, "c1");
    }
    let before = set_version(&root, 999);
    {
        let mut ws = env.workspace();
        ws.startup();
        assert_eq!(ws.status().state, DataRootState::Open, "the library opens");
        let store = knowledge_of(&ws);
        assert_eq!(store.state(), KnowledgeState::Incompatible);
        assert_eq!(store.status().state, KnowledgeState::Incompatible);
        assert_eq!(code(store.call(|_| ())), ErrorCode::KnowledgeUnavailable);
        assert_eq!(code(store.read(|_| ())), ErrorCode::KnowledgeUnavailable);
        assert_eq!(store.snapshot_if_due(), None);
        // The rest of the library works.
        assert!(sources::list(ws.require_open().unwrap()).is_ok());
    }
    assert_eq!(fs::read(root.join(KNOWLEDGE)).unwrap(), before);
}

#[test]
fn p3_04_a_store_atlas_cannot_read_is_left_alone() {
    let env = Env::new();
    let root = env.base.join("lib");
    env.workspace().choose(&root).unwrap();
    let path = root.join(KNOWLEDGE);
    for suffix in ["-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{suffix}", path.display()));
    }

    // Not SQLite at all.
    let garbage = b"these are not the notes you are looking for".repeat(200);
    fs::write(&path, &garbage).unwrap();
    {
        let mut ws = env.workspace();
        ws.startup();
        assert_eq!(ws.status().state, DataRootState::Open);
        assert_eq!(knowledge_of(&ws).state(), KnowledgeState::Damaged);
        assert_eq!(
            code(knowledge_of(&ws).call(|_| ())),
            ErrorCode::KnowledgeUnavailable
        );
    }
    assert_eq!(fs::read(&path).unwrap(), garbage);

    // SQLite, but someone else's.
    fs::remove_file(&path).unwrap();
    {
        let c = Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE recipes (name TEXT)").unwrap();
    }
    let foreign = fs::read(&path).unwrap();
    {
        let mut ws = env.workspace();
        ws.startup();
        assert_eq!(knowledge_of(&ws).state(), KnowledgeState::Damaged);
    }
    assert_eq!(fs::read(&path).unwrap(), foreign);
}

// ----------------------------------------------------------- P3-06, rule

fn snapshots(root: &Path) -> Vec<snapshot::Snapshot> {
    snapshot::list(&root.join(SNAPSHOTS_DIR))
}

fn collections_in(path: &Path) -> i64 {
    Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap()
        .query_row("SELECT count(*) FROM collections", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn p3_06_a_snapshot_when_changed_at_most_daily_keeping_fourteen() {
    let env = Env::new();
    let root = env.base.join("lib");
    let clock = Arc::new(TestClock::at(datetime!(2026-11-11 09:00 UTC)));
    let day = time::Duration::hours(24);
    let ws_at =
        |clock: &Arc<TestClock>| env.workspace().with_clock(clock.clone() as Arc<dyn Clock>);

    let mut ws = ws_at(&clock);
    ws.choose(&root).unwrap();
    let store = knowledge_of(&ws);
    // Opening a new library: nothing to keep yet.
    assert_eq!(
        store.status().snapshots.last_outcome,
        Some(SnapshotOutcome::NotChanged)
    );
    assert!(snapshots(&root).is_empty());

    add_collection(&ws, "c1");
    assert_eq!(store.snapshot_if_due(), Some(SnapshotOutcome::Taken));
    assert_eq!(store.snapshot_if_due(), Some(SnapshotOutcome::NotChanged));
    add_collection(&ws, "c2");
    assert_eq!(store.snapshot_if_due(), Some(SnapshotOutcome::NotDue));
    clock.advance(time::Duration::hours(23));
    assert_eq!(store.snapshot_if_due(), Some(SnapshotOutcome::NotDue));
    clock.advance(time::Duration::hours(1));
    assert_eq!(store.snapshot_if_due(), Some(SnapshotOutcome::Taken));
    let snaps = snapshots(&root);
    assert_eq!(snaps.len(), 2);
    assert_eq!(collections_in(&snaps[0].path), 1);
    assert_eq!(collections_in(&snaps[1].path), 2);
    let status = store.status().snapshots;
    assert_eq!(status.count, 2);
    assert_eq!(status.bytes, snaps.iter().map(|s| s.bytes).sum::<u64>());
    assert_eq!(status.newest_at.as_deref(), Some("2026-11-12T09:00:00Z"));
    drop(ws);

    // A change, a day, and the next open takes one.
    clock.advance(day);
    {
        let mut ws = ws_at(&clock);
        ws.startup();
        add_collection(&ws, "c3");
    }
    clock.advance(day);
    {
        let mut ws = ws_at(&clock);
        ws.startup();
        assert_eq!(
            knowledge_of(&ws).status().snapshots.last_outcome,
            Some(SnapshotOutcome::Taken)
        );
    }
    assert_eq!(snapshots(&root).len(), 3);

    // Day after day: the fifteenth snapshot removes the oldest, and only
    // after it is in place.
    let mut ws = ws_at(&clock);
    ws.startup();
    let first = snapshots(&root)[0].path.clone();
    for n in 0..11 {
        clock.advance(day);
        add_collection(&ws, &format!("d{n}"));
        assert_eq!(
            knowledge_of(&ws).snapshot_if_due(),
            Some(SnapshotOutcome::Taken)
        );
    }
    assert_eq!(snapshots(&root).len(), snapshot::KEEP);
    assert!(first.exists(), "fourteen are kept");
    clock.advance(day);
    add_collection(&ws, "last");
    assert_eq!(
        knowledge_of(&ws).snapshot_if_due(),
        Some(SnapshotOutcome::Taken)
    );
    let snaps = snapshots(&root);
    assert_eq!(snaps.len(), snapshot::KEEP);
    assert!(!first.exists(), "the oldest went");
    assert_eq!(collections_in(&snaps.last().unwrap().path), 15);
    for s in &snaps {
        assert!(snapshot::verify(&s.path), "{}", s.path.display());
    }
    assert_eq!(
        fs::read_dir(root.join(SNAPSHOT_STAGING_DIR))
            .unwrap()
            .count(),
        0
    );
}

// ----------------------------------------------------------------- P3-08

fn generate(dir: &Path) {
    let out = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
        .arg(dir)
        .args(["--files", "4", "--text-kb", "8"])
        .output()
        .expect("node runs the fixture generator");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn find(dir: &Path, name: &str) -> PathBuf {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| n == name) {
                return p;
            }
            if e.file_type().unwrap().is_dir() && p.file_name().unwrap() != "locked" {
                stack.push(p);
            }
        }
    }
    panic!("{name} not generated")
}

struct Lib {
    ws: Workspace,
    _h: ExtractionHarness,
}

impl Lib {
    fn root(&self) -> &OpenRoot {
        self.ws.require_open().unwrap()
    }

    fn settled(&self) -> bool {
        let inv = inventory::control::overview(self.root()).unwrap();
        let ov = self
            .root()
            .reader()
            .read(|c| db::overview(c, None))
            .unwrap()
            .unwrap();
        inv.active.is_none() && ov.active.is_none() && ov.queued_jobs == 0 && ov.counts.pending == 0
    }

    fn rescan(&self, id: &str) {
        inventory::control::enqueue(self.root(), id).unwrap();
        // The scan is queued before it runs: wait for it to start or end.
        std::thread::sleep(Duration::from_millis(200));
        assert!(wait_until(LONG, || self.settled()), "never settled");
    }

    /// Resolves on a read-only connection and checks nothing was written.
    fn resolve(
        &self,
        a: &atlas_lib::domain::dto::Anchor,
    ) -> atlas_lib::domain::dto::ResolvedAnchor {
        self.root()
            .reader()
            .read(|c| {
                let before = c.total_changes();
                let r = anchor::resolve(c, a).unwrap();
                assert_eq!(c.total_changes(), before, "resolution never writes");
                r
            })
            .unwrap()
    }

    fn file_id(&self, display_name: &str) -> String {
        let name = display_name.to_owned();
        self.root()
            .reader()
            .read(move |c| {
                c.query_row(
                    "SELECT id FROM file_instances WHERE display_name = ?1 AND availability = 'available'",
                    [name],
                    |r| r.get(0),
                )
            })
            .unwrap()
            .unwrap()
    }
}

#[test]
fn p3_08_anchors_resolve_against_the_catalog_now() {
    let env = Env::new();
    let generated = env.base.join("generated");
    generate(&generated);
    let books = env.base.join("Books");
    let elsewhere = env.base.join("Elsewhere");
    fs::create_dir_all(books.join("sub")).unwrap();
    fs::create_dir_all(&elsewhere).unwrap();
    let source = find(&generated, "book 000001.pdf");
    fs::copy(&source, books.join("a.pdf")).unwrap();

    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = env.workspace_with_extraction(&h);
    ws.choose(&env.base.join("AtlasData")).unwrap();
    let lib = Lib { ws, _h: h };
    let books_id = sources::add(lib.root(), &books).unwrap().id;
    let elsewhere_id = sources::add(lib.root(), &elsewhere).unwrap().id;
    lib.rescan(&books_id);
    lib.rescan(&elsewhere_id);

    // The last passage of the document, by value.
    let first_file = lib.file_id("a.pdf");
    let fid = first_file.clone();
    let last: u32 = lib
        .root()
        .reader()
        .read(move |c| {
            c.query_row(
                "SELECT max(p.ordinal) FROM passages p
                 JOIN file_instances fi ON fi.document_id = p.document_id WHERE fi.id = ?1",
                [fid],
                |r| r.get(0),
            )
        })
        .unwrap()
        .unwrap();
    let place = lib
        .root()
        .reader()
        .read(|c| anchor::build(c, &first_file, Place::Ordinal(last)).unwrap())
        .unwrap()
        .expect("an anchor");
    assert!(place.excerpt.as_ref().is_some_and(|e| !e.is_empty()));
    assert!(place.page_no.is_some(), "a PDF passage has a page");
    let whole = lib
        .root()
        .reader()
        .read(|c| anchor::build(c, &first_file, Place::Document).unwrap())
        .unwrap()
        .unwrap();
    let page = lib
        .root()
        .reader()
        .read(|c| anchor::build(c, &first_file, Place::Page(place.page_no.unwrap())).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(page.page_no, place.page_no);

    let r = lib.resolve(&place);
    assert_eq!((r.state, r.excerpt_found), (AnchorState::Found, true));
    assert_eq!(r.file_instance_id.as_deref(), Some(first_file.as_str()));
    assert_eq!(lib.resolve(&whole).state, AnchorState::Found);
    assert_eq!(lib.resolve(&page).state, AnchorState::Found);

    // Moved into a subfolder: found, at the new file.
    fs::rename(books.join("a.pdf"), books.join("sub/a.pdf")).unwrap();
    lib.rescan(&books_id);
    let r = lib.resolve(&place);
    assert_eq!(r.state, AnchorState::Found);
    let moved = r.file_instance_id.clone().unwrap();
    assert_ne!(moved, first_file, "a moved file is a new row");

    // Renamed: found.
    fs::rename(books.join("sub/a.pdf"), books.join("sub/renamed.pdf")).unwrap();
    lib.rescan(&books_id);
    let r = lib.resolve(&place);
    assert_eq!(r.state, AnchorState::Found);
    assert_eq!(r.file_instance_id.unwrap(), lib.file_id("renamed.pdf"));

    // Only a copy elsewhere: found, there.
    fs::copy(books.join("sub/renamed.pdf"), elsewhere.join("copy.pdf")).unwrap();
    fs::remove_file(books.join("sub/renamed.pdf")).unwrap();
    lib.rescan(&books_id);
    lib.rescan(&elsewhere_id);
    let r = lib.resolve(&place);
    assert_eq!(r.state, AnchorState::Found);
    assert_eq!(r.file_instance_id.unwrap(), lib.file_id("copy.pdf"));

    // A catalog rebuilt: the same content under another id is found by
    // its hash; content the catalog doesn't know is unknown.
    let mut rebuilt = place.clone();
    rebuilt.document_id = "not-this-id".into();
    let r = lib.resolve(&rebuilt);
    assert_eq!(r.state, AnchorState::Found);
    assert_eq!(r.document_id.as_deref(), Some(place.document_id.as_str()));
    let mut stranger = place.clone();
    stranger.document_id = "not-this-id".into();
    stranger.content_hash = "00".repeat(32);
    let r = lib.resolve(&stranger);
    assert_eq!((r.state, r.file_instance_id), (AnchorState::Unknown, None));
    assert_eq!(r.ordinal, place.ordinal, "the saved place");

    // An excerpt that is nowhere: the saved place, said so.
    let mut lost = place.clone();
    lost.excerpt = Some("zebra quantum marmalade never written".into());
    let r = lib.resolve(&lost);
    assert_eq!((r.state, r.excerpt_found), (AnchorState::Found, false));
    assert_eq!((r.ordinal, r.page_no), (place.ordinal, place.page_no));

    // Re-read by a newer reader, the passages re-split (simulated by
    // shifting every ordinal and page): found elsewhere.
    let doc = place.document_id.clone();
    lib.root()
        .catalog()
        .call(move |c| {
            c.execute(
                "UPDATE passages SET ordinal = ordinal + 1000, page_no = page_no + 1
                 WHERE document_id = ?1",
                [doc],
            )
        })
        .unwrap()
        .unwrap();
    let r = lib.resolve(&place);
    assert_eq!(
        (r.state, r.excerpt_found),
        (AnchorState::FoundElsewhere, true)
    );
    assert_eq!(r.ordinal, Some(last + 1000));
    assert_eq!(r.page_no, place.page_no.map(|p| p + 1));

    // The file's bytes changed: its document has no file now.
    fs::copy(
        find(&generated, "book 000002.pdf"),
        elsewhere.join("copy.pdf"),
    )
    .unwrap();
    lib.rescan(&elsewhere_id);
    let r = lib.resolve(&place);
    assert_eq!((r.state, r.file_instance_id), (AnchorState::NoFile, None));

    // Back again, then the folder is removed from the library: no file.
    fs::copy(&source, elsewhere.join("back.pdf")).unwrap();
    lib.rescan(&elsewhere_id);
    assert_eq!(lib.resolve(&place).state, AnchorState::FoundElsewhere);
    sources::remove(lib.root(), &elsewhere_id).unwrap();
    assert!(wait_until(LONG, || lib.settled()));
    assert_eq!(lib.resolve(&place).state, AnchorState::NoFile);
}
