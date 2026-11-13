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
            "SELECT COUNT(*) FROM documents d WHERE extractor_version <> CASE
                 (SELECT f.file_type FROM file_instances f WHERE f.document_id = d.id LIMIT 1)
                 WHEN 'epub' THEN 3 ELSE 2 END"
        ),
        0,
        "PDFs at reader version 2, EPUBs at 3"
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

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../spikes/extraction-bench/corpus")
}

/// Whether `text` holds Project Gutenberg's markers or licence. (A note
/// the publisher put inside the book, after START, such as The Raven's
/// "This ebook was one of Project Gutenberg's early files", is the book's.)
fn has_boilerplate(text: &str) -> bool {
    let t = text.to_lowercase();
    [
        "start of the project gutenberg",
        "end of the project gutenberg",
        "project gutenberg license",
        "literary archive foundation",
    ]
    .iter()
    .any(|m| t.contains(m))
}

/// The passages of the documents whose file name starts with `stem`.
fn texts(c: &Connection, stem: &str) -> Vec<String> {
    c.prepare(
        "SELECT p.text FROM passages p
         WHERE p.document_id IN (SELECT document_id FROM file_instances WHERE display_name LIKE ?1)
         ORDER BY p.ordinal",
    )
    .unwrap()
    .query_map([format!("{stem}%")], |r| r.get(0))
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// P3-03: Project Gutenberg's header and licence are left out when an EPUB
/// is read (reader version 3), and counted. A book read by version 2 is read
/// again only if its text mentions Project Gutenberg, staying searchable
/// meanwhile; other EPUBs move to version 3 without being read.
#[test]
fn p3_03_gutenberg_boilerplate_is_left_out_and_only_those_books_are_read_again() {
    let env = Env::new();
    let books = env.base.join("Books");
    fixture(&books);
    for name in ["pg11.epub", "pg1065.epub"] {
        fs::copy(corpus().join("epub").join(name), books.join(name)).unwrap();
    }

    // Read by the current readers.
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = env.workspace_with_extraction(&h);
    ws.choose(&env.base.join("AtlasData")).unwrap();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &books).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    assert!(
        wait_until(LONG, || settled(&ws)),
        "first read never settled"
    );
    drop(ws);

    let c = Connection::open(catalog(&env)).unwrap();
    for stem in ["pg11", "pg1065"] {
        let all = texts(&c, stem).join("\n");
        assert!(!all.is_empty(), "{stem} has text");
        assert!(!has_boilerplate(&all), "{stem}: no header or licence left");
    }
    assert!(
        texts(&c, "pg11")[0].contains("Wonderland"),
        "Alice starts at its title page, after the header"
    );
    let marked = count(
        &c,
        "SELECT COUNT(*) FROM documents d WHERE d.boilerplate_chars > 0",
    );
    assert_eq!(
        marked, 2,
        "both Project Gutenberg books counted what was cut"
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM documents d WHERE d.boilerplate_chars > 0 AND d.id IN
                 (SELECT document_id FROM file_instances WHERE display_name LIKE 'book %')"
        ),
        0,
        "the generated books have nothing cut"
    );
    let epubs = count(
        &c,
        "SELECT COUNT(DISTINCT document_id) FROM file_instances WHERE file_type = 'epub'",
    );
    assert!(epubs > 2, "generated EPUBs as well");

    // As reader version 2 left them: every EPUB at 2, the two books with
    // their licence still in their text.
    c.execute_batch(
        "UPDATE documents SET extractor_version = 2, boilerplate_chars = 0
         WHERE id IN (SELECT document_id FROM file_instances WHERE file_type = 'epub');
         INSERT INTO passages (document_id, ordinal, page_no, location, text)
         SELECT d.id, (SELECT MAX(ordinal) + 1 FROM passages WHERE document_id = d.id), NULL,
                'Chapter 99, part 1',
                '*** END OF THE PROJECT GUTENBERG EBOOK *** Updated editions will replace the previous one. The Project Gutenberg License applies.'
         FROM documents d WHERE d.id IN
             (SELECT document_id FROM file_instances WHERE display_name LIKE 'pg%');",
    )
    .unwrap();
    drop(c);

    // Opened without a reader: only the two are due, the others move to 3,
    // and the books are still found.
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = env.workspace_with_inventory(h.inv.deps.clone());
    ws.startup();
    let root = ws.require_open().unwrap();
    let c = Connection::open(catalog(&env)).unwrap();
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM documents WHERE refresh = 1"),
        2,
        "only the Project Gutenberg books are read again"
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM documents d WHERE refresh = 0 AND extractor_version = 3
               AND d.id IN (SELECT document_id FROM file_instances WHERE file_type = 'epub')"
        ),
        epubs - 2,
        "the other EPUBs move to version 3 without being read"
    );
    let found = search::run(root.reader(), root.root_status(), &query("Wonderland")).unwrap();
    assert!(!found.hits.is_empty(), "still searchable while due");
    drop(ws);

    // Opened with a reader: exactly those two are read, and the licence goes.
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = env.workspace_with_extraction(&h);
    ws.startup();
    assert!(wait_until(LONG, || settled(&ws)), "refresh never settled");
    assert_eq!(WorkerStats::get(&h.deps.stats.extracts), 2);
    for stem in ["pg11", "pg1065"] {
        assert!(!has_boilerplate(&texts(&c, stem).join("\n")), "{stem}");
    }
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM documents WHERE boilerplate_chars > 0"
        ),
        2
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM documents WHERE refresh = 1"),
        0
    );
}

/// P4-04 (Slice 4.1): PDFs marked Needs OCR before page counts were kept
/// are read once more for them, and nothing else is read again.
#[test]
fn p4_04_scanned_pdfs_marked_before_counts_are_read_once_more() {
    let env = Env::new();
    let books = env.base.join("Books");
    fs::create_dir_all(&books).unwrap();
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../spikes/extraction-bench/corpus");
    for rel in [
        "pdf/scanned/ia-alice-1894.pdf",
        "pdf/scanned/synthetic-scan.pdf",
        "pdf/generated/no-tounicode.pdf",
        "pdf/generated/book.pdf",
    ] {
        let from = corpus.join(rel);
        fs::copy(&from, books.join(from.file_name().unwrap())).unwrap();
    }

    // Read now, then made to look as a catalog of version 14 left them.
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = env.workspace_with_extraction(&h);
    ws.choose(&env.base.join("AtlasData")).unwrap();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &books).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    assert!(
        wait_until(LONG, || settled(&ws)),
        "first read never settled"
    );
    drop(ws);
    let c = Connection::open(catalog(&env)).unwrap();
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM documents WHERE state = 'needs_ocr' AND page_count IS NOT NULL"
        ),
        3,
        "counted when read"
    );
    c.execute_batch(
        "UPDATE documents SET page_count = NULL WHERE state = 'needs_ocr';
         ALTER TABLE documents DROP COLUMN text_pages;
         DROP TABLE scan_problems;
         DROP TABLE scan_missing;
         ALTER TABLE jobs DROP COLUMN origin;
         ALTER TABLE jobs DROP COLUMN new_files;
         ALTER TABLE jobs DROP COLUMN changed_files;
         ALTER TABLE jobs DROP COLUMN missing_files;
         DELETE FROM schema_migrations WHERE version >= 15;",
    )
    .unwrap();
    drop(c);

    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = env.workspace_with_extraction(&h);
    ws.startup();
    assert!(
        wait_until(LONG, || settled(&ws)),
        "the re-read never settled"
    );
    drop(ws);
    assert_eq!(
        WorkerStats::get(&h.deps.stats.extracts),
        3,
        "only the three scanned PDFs were read again"
    );
    let c = Connection::open(catalog(&env)).unwrap();
    let rows: Vec<(String, Option<i64>, Option<i64>, i64)> = c
        .prepare(
            "SELECT f.display_name, d.page_count, d.text_pages, d.refresh
               FROM documents d JOIN file_instances f ON f.document_id = d.id
              WHERE d.state = 'needs_ocr' ORDER BY f.display_name",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        vec![
            ("ia-alice-1894.pdf".into(), Some(4), Some(0), 0),
            ("no-tounicode.pdf".into(), Some(3), Some(0), 0),
            ("synthetic-scan.pdf".into(), Some(3), Some(0), 0),
        ]
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM documents WHERE state = 'done'"),
        1,
        "the book is still read"
    );
}
