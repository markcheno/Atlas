//! Slice 3.2: kills (phase3-spec §12).
//!
//! - P3-05: a kill during a trail-step write (raw, and through the
//!   recorder since Slice 3.3), a bookmark write and a collection change
//!   leaves whole rows only; `integrity_check` passes; the catalog is
//!   unaffected.
//! - P3-06 (the kill): a kill during snapshots leaves no partial snapshot,
//!   and the next open empties the staging folder.
//!
//! The child halves are this test binary run again with `--ignored`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use atlas_lib::domain::dto::{
    DataRootState, KnowledgeState, SearchIntent, SearchPage, SearchQuery, SemanticStatus,
};
use atlas_lib::services::knowledge::{SNAPSHOT_STAGING_DIR, SNAPSHOTS_DIR, snapshot};
use atlas_lib::services::workspace::layout::{CATALOG, KNOWLEDGE};
use atlas_lib::services::workspace::{Workspace, WorkspaceConfig};
use rusqlite::{Connection, OpenFlags, params};

mod common;
use common::{local, wait_until};

const ROOT_ENV: &str = "ATLAS_TEST_KNOWLEDGE_CRASH_ROOT";
const KILLS: u64 = 6;

fn workspace(base: &Path) -> Workspace {
    Workspace::new(
        WorkspaceConfig {
            config_dir: base.join("config"),
            default_root: base.join("default"),
        },
        local(),
    )
}

/// A new library at `base/lib`, closed again.
fn create(base: &Path) -> PathBuf {
    let root = base.join("lib");
    workspace(base).choose(&root).unwrap();
    root
}

fn count(path: &Path, sql: &str) -> i64 {
    // Read-only, through the log, while the child may still be writing.
    let Ok(c) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return 0;
    };
    c.query_row(sql, [], |r| r.get(0)).unwrap_or(0)
}

fn spawn(test: &str, root: &Path) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--ignored", "--nocapture"])
        .env(ROOT_ENV, root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// The library opens, and both databases pass their checks.
fn assert_sound(base: &Path, root: &Path) {
    let mut ws = workspace(base);
    ws.open(root).unwrap();
    assert_eq!(ws.status().state, DataRootState::Open);
    let open = ws.require_open().unwrap();
    assert_eq!(open.knowledge().state(), KnowledgeState::Ready);
    let ok: String = open
        .knowledge()
        .call(|c| c.query_row("PRAGMA integrity_check", [], |r| r.get(0)))
        .unwrap()
        .unwrap();
    assert_eq!(ok, "ok", "the knowledge store");
    open.catalog()
        .call(|c| {
            let ok: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
            assert_eq!(ok, "ok", "the catalog");
            c.execute_batch(
                "INSERT INTO passages_fts (passages_fts, rank) VALUES ('integrity-check', 1)",
            )
        })
        .unwrap()
        .unwrap();
}

#[test]
fn p3_05_a_kill_during_writes_leaves_whole_rows() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let root = create(base);
    let store = root.join(KNOWLEDGE);
    for kill in 0..KILLS {
        let before = count(&store, "SELECT count(*) FROM trails");
        let mut child = spawn("p3_05_child_process", &root);
        assert!(
            wait_until(Duration::from_secs(120), || count(
                &store,
                "SELECT count(*) FROM trails"
            ) > before + 3),
            "the child never wrote"
        );
        std::thread::sleep(Duration::from_millis(3 + kill * 17));
        child.kill().unwrap();
        child.wait().unwrap();

        assert_sound(base, &root);
        let c = Connection::open(&store).unwrap();
        let q = |sql: &str| -> i64 { c.query_row(sql, [], |r| r.get(0)).unwrap() };
        // Each trail went in with its steps, or not at all: the raw ones
        // with three, the recorder's with its count kept in step.
        assert_eq!(
            q("SELECT count(*) FROM trails t
               WHERE t.step_count != (SELECT count(*) FROM trail_steps s WHERE s.trail_id = t.id)"),
            0
        );
        assert_eq!(
            q(
                "SELECT count(*) FROM trail_steps s JOIN trails t ON t.id = s.trail_id
               WHERE t.started_at = 't'
                 AND NOT EXISTS (SELECT 1 FROM step_documents d WHERE d.step_id = s.id)"
            ),
            0
        );
        assert!(
            q("SELECT count(*) FROM trail_steps WHERE kind = 'search' AND at != 't'") > 0,
            "the recorder wrote steps too"
        );
        assert_eq!(q("SELECT count(*) FROM bookmarks WHERE note IS NULL"), 0);
        // Each collection with its one item.
        assert_eq!(
            q("SELECT count(*) FROM collections c
               WHERE (SELECT count(*) FROM collection_items i WHERE i.collection_id = c.id) != 1"),
            0
        );
        let rows = q(
            "SELECT (SELECT count(*) FROM trails) + (SELECT count(*) FROM trail_steps)
                    + (SELECT count(*) FROM step_documents) + (SELECT count(*) FROM bookmarks)
                    + (SELECT count(*) FROM collections) + (SELECT count(*) FROM collection_items)",
        );
        assert!(
            q("SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'changes'") >= rows,
            "every row was counted as a change"
        );
    }
}

fn query(words: &str) -> SearchQuery {
    SearchQuery {
        text: words.into(),
        source_root_id: None,
        file_type: None,
        include_missing: false,
        page: 0,
        page_size: 20,
    }
}

fn empty_page() -> SearchPage {
    SearchPage {
        hits: Vec::new(),
        page: 0,
        page_size: 20,
        matched_documents: 0,
        capped: false,
        reading_text: false,
        text_read_files: 0,
        total_files: 0,
        semantic: SemanticStatus::Off,
    }
}

/// Child half: trails with steps, bookmarks and collections, until killed.
#[test]
#[ignore = "run only as the child of p3_05_a_kill_during_writes_leaves_whole_rows"]
fn p3_05_child_process() {
    let root = PathBuf::from(std::env::var(ROOT_ENV).expect("the parent sets the root"));
    let base = root.parent().unwrap().to_path_buf();
    let mut ws = workspace(&base);
    ws.open(&root).unwrap();
    let store = ws.require_open().unwrap().knowledge();
    let until = Instant::now() + Duration::from_secs(300);
    let run = format!("{}", std::process::id());
    let mut n = 0u64;
    while Instant::now() < until {
        let id = format!("{run}-{n}");
        // A step through the recorder: the trail, its step, its counts
        // and the session in one transaction.
        ws.require_open().unwrap().recorder().search(
            &query(&format!("search {id}")),
            SearchIntent::Submit,
            &empty_page(),
        );
        let note = "a note ".repeat(200);
        store
            .call(move |c| {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO trails (id, step_count, started_at, last_step_at)
                     VALUES (?1, 3, 't', 't')",
                    [&id],
                )?;
                for s in 0..3 {
                    let step = format!("{id}/{s}");
                    tx.execute(
                        "INSERT INTO trail_steps (id, trail_id, seq, kind, at, payload)
                         VALUES (?1, ?2, ?3, 'search', 't', ?4)",
                        params![step, id, s, "{\"words\":\"birds\"}".repeat(50)],
                    )?;
                    tx.execute(
                        "INSERT INTO step_documents (step_id, document_id, content_hash)
                         VALUES (?1, 'doc', x'00')",
                        [&step],
                    )?;
                }
                tx.commit()?;
                c.execute(
                    "INSERT INTO bookmarks (id, anchor, document_id, content_hash, note, created_at, updated_at)
                     VALUES (?1, '{}', 'doc', x'00', ?2, 't', 't')",
                    params![id, note],
                )?;
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO collections (id, kind, name, created_at, updated_at)
                     VALUES (?1, 'project', 'Thesis', 't', 't')",
                    [&id],
                )?;
                tx.execute(
                    "INSERT INTO collection_items (collection_id, item_kind, item_id, added_at)
                     VALUES (?1, 'trail', ?1, 't')",
                    [&id],
                )?;
                tx.commit()
            })
            .unwrap()
            .unwrap();
        n += 1;
    }
}

#[test]
fn p3_06_a_kill_during_a_snapshot_leaves_no_partial_file() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let root = create(base);
    let snaps = root.join(SNAPSHOTS_DIR);
    let staging = root.join(SNAPSHOT_STAGING_DIR);
    for kill in 0..KILLS {
        let before = snapshot::list(&snaps).len();
        let mut child = spawn("p3_06_child_process", &root);
        assert!(
            wait_until(Duration::from_secs(120), || {
                let n = snapshot::list(&snaps).len();
                n > before || n >= snapshot::KEEP
            }),
            "the child never took a snapshot"
        );
        std::thread::sleep(Duration::from_millis(5 + kill * 29));
        child.kill().unwrap();
        child.wait().unwrap();

        // Only whole, sound snapshots under their final names.
        for e in fs::read_dir(&snaps).unwrap().flatten() {
            let name = e.file_name().into_string().unwrap();
            if e.path() == staging {
                continue;
            }
            assert!(
                snapshot::parse_name(&name).is_some(),
                "only snapshots: {name}"
            );
            assert!(snapshot::verify(&e.path()), "{name} passes its check");
        }
        // A kill between the rename and the pruning leaves one extra: the
        // oldest goes only once a newer one is in place, at the next snapshot.
        assert!(snapshot::list(&snaps).len() <= snapshot::KEEP + 1);
        assert_sound(base, &root);
        assert_eq!(
            fs::read_dir(&staging).unwrap().count(),
            0,
            "opening empties the staging folder"
        );
    }
    // The catalog was never part of it.
    assert!(root.join(CATALOG).is_file());
}

/// Child half: a change and a snapshot, over and over, until killed.
#[test]
#[ignore = "run only as the child of p3_06_a_kill_during_a_snapshot_leaves_no_partial_file"]
fn p3_06_child_process() {
    let root = PathBuf::from(std::env::var(ROOT_ENV).expect("the parent sets the root"));
    let base = root.parent().unwrap().to_path_buf();
    let mut ws = workspace(&base);
    ws.open(&root).unwrap();
    let store = ws.require_open().unwrap().knowledge();
    let until = Instant::now() + Duration::from_secs(300);
    let run = std::process::id();
    let mut n = 0u64;
    while Instant::now() < until {
        let id = format!("{run}-{n}");
        store
            .call(move |c| {
                c.execute(
                    "INSERT INTO findings (id, kind, text, sources, created_at, updated_at)
                     VALUES (?1, 'passage', ?2, '[]', 't', 't')",
                    params![id, "a finding worth keeping ".repeat(400)],
                )
            })
            .unwrap()
            .unwrap();
        snapshot::take(store.inner());
        n += 1;
    }
}
