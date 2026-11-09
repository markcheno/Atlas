//! Slice 1.6: reading again with a newer reader without losing search
//! (phase1-spec §6.1). A library read by the version-1 readers (EPUB text
//! with `# ` headings; no first-page titles) is upgraded on open: EPUBs and
//! untitled PDFs are read again while their old text stays searchable and
//! their files stay "Text ready"; PDFs whose metadata already has a title
//! are not read again.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use atlas_lib::domain::dto::{ExtractionState, SearchQuery};
use atlas_lib::services::extraction::db;
use atlas_lib::services::extraction::supervisor::WorkerStats;
use atlas_lib::services::extraction::worker_main::TEST_FAULT_ENV;
use atlas_lib::services::workspace::Workspace;
use atlas_lib::services::{documents, inventory, search, sources};
use rusqlite::Connection;

mod common;
use common::{Env, ExtractionHarness, fast_extraction, originals, wait_until, worker};

const LONG: Duration = Duration::from_secs(180);

fn fixture(dir: &Path) {
    let out = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
        .arg(dir)
        .args(["--files", "6", "--text-kb", "4"])
        .output()
        .expect("node runs the fixture generator");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn settled(ws: &Workspace) -> bool {
    let root = ws.require_open().unwrap();
    let inv = inventory::control::overview(root).unwrap();
    let ov = root
        .reader()
        .read(|c| db::overview(c, None))
        .unwrap()
        .unwrap();
    inv.active.is_none()
        && ov.active.is_none()
        && ov.queued_jobs == 0
        && ov.counts.pending == 0
        && ov.counts.updating == 0
}

fn catalog(env: &Env) -> PathBuf {
    env.base.join("AtlasData/library/catalog.sqlite")
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        source_root_id: None,
        file_type: None,
        include_missing: false,
        page: 0,
        page_size: 20,
    }
}

/// Reads `books` with the current readers, then rewrites the catalog as the
/// version-1 readers left it.
fn read_then_age(env: &Env, books: &Path) {
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = env.workspace_with_extraction(&h);
    ws.choose(&env.base.join("AtlasData")).unwrap();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, books).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    assert!(
        wait_until(LONG, || settled(&ws)),
        "first read never settled"
    );
    drop(ws);

    let c = Connection::open(catalog(env)).unwrap();
    c.execute_batch(
        "UPDATE documents SET extractor_version = 1;
         -- What html2text's plain decorator wrote for an EPUB heading.
         UPDATE passages SET text = '# ' || text
         WHERE document_id IN (SELECT document_id FROM file_instances WHERE file_type = 'epub');
         -- One PDF read when the title rules were older: no usable title.
         UPDATE documents SET title = 'Untitled', content_title = NULL
         WHERE id = (SELECT document_id FROM file_instances
                     WHERE display_name = 'book 000001.pdf');",
    )
    .unwrap();
}

#[test]
fn an_older_reader_is_replaced_while_the_text_stays_searchable() {
    let env = Env::new();
    let books = env.base.join("Books");
    fixture(&books);
    read_then_age(&env, &books);
    let before = originals(&books);
    let c = Connection::open(catalog(&env)).unwrap();
    let epubs = count(
        &c,
        "SELECT COUNT(DISTINCT document_id) FROM file_instances WHERE file_type = 'epub'",
    );
    assert!(epubs >= 2);
    assert!(count(&c, "SELECT COUNT(*) FROM passages WHERE text LIKE '# %'") > 0);
    drop(c);

    // Reopened without a reader: the old text is still found, the files are
    // still "Text ready", and the status says it is being updated.
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = env.workspace_with_inventory(h.inv.deps.clone());
    ws.startup();
    let root = ws.require_open().unwrap();
    let c = Connection::open(catalog(&env)).unwrap();
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM documents WHERE refresh = 1"),
        epubs + 1,
        "every EPUB and the untitled PDF are due"
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM documents d WHERE refresh = 0 AND extractor_version = 2
               AND EXISTS (SELECT 1 FROM file_instances f
                           WHERE f.document_id = d.id AND f.file_type = 'pdf')"
        ),
        count(
            &c,
            "SELECT COUNT(DISTINCT document_id) FROM file_instances WHERE file_type = 'pdf'"
        ) - 1,
        "titled PDFs move to version 2 without being read"
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM file_instances
             WHERE document_id IS NULL OR extract_state <> 'linked'"
        ),
        0,
        "nothing is unlinked"
    );
    let found = search::run(
        root.reader(),
        root.root_status(),
        &query("\"Astronomy marker 0\""),
    )
    .unwrap();
    assert_eq!(found.hits.len(), 1, "old text still searchable");
    let epub_file = found.hits[0].file_instance_id.clone();
    let d = documents::detail(root.reader(), root.root_status(), &epub_file).unwrap();
    assert_eq!(d.file.extraction.state, ExtractionState::Done);
    let ov = root
        .reader()
        .read(|c| db::overview(c, None))
        .unwrap()
        .unwrap();
    assert_eq!(ov.counts.pending, 0);
    assert!(ov.counts.updating as i64 > epubs);
    drop(ws);

    // Reopened with a reader: only the due documents are read again.
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = env.workspace_with_extraction(&h);
    ws.startup();
    assert!(wait_until(LONG, || settled(&ws)), "refresh never settled");
    assert_eq!(
        WorkerStats::get(&h.deps.stats.extracts) as i64,
        epubs + 1,
        "only EPUBs and the untitled PDF are read again"
    );
    let root = ws.require_open().unwrap();
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM passages WHERE text LIKE '#%'"),
        0
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM documents WHERE refresh = 1"),
        0
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM documents WHERE extractor_version <> 2"
        ),
        0
    );
    let title: String = c
        .query_row(
            "SELECT d.title FROM documents d JOIN file_instances f ON f.document_id = d.id
             WHERE f.display_name = 'book 000001.pdf'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(title, "Engineering book 000001", "the title is read again");
    let found = search::run(
        root.reader(),
        root.root_status(),
        &query("\"Astronomy marker 0\""),
    )
    .unwrap();
    assert_eq!(found.hits.len(), 1);
    assert!(
        found.hits[0].snippet.iter().all(|s| !s.text.contains("# ")),
        "{:?}",
        found.hits[0].snippet
    );
    ws.require_open()
        .unwrap()
        .catalog()
        .call(|w| {
            w.execute_batch(
                "INSERT INTO passages_fts (passages_fts, rank) VALUES ('integrity-check', 1)",
            )
        })
        .unwrap()
        .expect("the full-text index matches the passages");
    drop(ws);
    assert_eq!(originals(&books), before);
}

#[test]
fn a_refresh_that_keeps_crashing_keeps_the_old_text() {
    let env = Env::new();
    let books = env.base.join("Books");
    let generated = env.base.join("generated");
    fixture(&generated);
    fs::create_dir_all(&books).unwrap();
    fs::copy(
        generated.join("Astronomy/Café crème book 000000.epub"),
        books.join("crash-exit.epub"),
    )
    .unwrap();
    read_then_age(&env, &books);
    let c = Connection::open(catalog(&env)).unwrap();
    let passages = count(&c, "SELECT COUNT(*) FROM passages");

    // Every attempt to read it again crashes the worker.
    let h = ExtractionHarness::new(
        fast_extraction(),
        worker().env(TEST_FAULT_ENV, "crash-exit:exit"),
    );
    let mut ws = env.workspace_with_extraction(&h);
    ws.startup();
    assert!(wait_until(LONG, || settled(&ws)), "never settled");
    assert!(WorkerStats::get(&h.deps.stats.kills) >= 2);
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM documents WHERE refresh = 1"),
        0
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM documents WHERE extractor_version = 1"
        ),
        1,
        "still the old reader's text"
    );
    assert_eq!(count(&c, "SELECT COUNT(*) FROM passages"), passages);
    let root = ws.require_open().unwrap();
    let found = search::run(
        root.reader(),
        root.root_status(),
        &query("\"Astronomy marker 0\""),
    )
    .unwrap();
    assert_eq!(found.hits.len(), 1, "still searchable");
    let d = documents::detail(
        root.reader(),
        root.root_status(),
        &found.hits[0].file_instance_id,
    )
    .unwrap();
    assert_eq!(d.file.extraction.state, ExtractionState::Done);
}
