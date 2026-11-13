//! Slice 4.5.1: one book in two formats (ADR 0023). A PDF and an EPUB in one
//! folder with one name whose texts match are one book: it is indexed for
//! meaning once, found once, listed once, and never related to itself; the
//! owner can say otherwise. A library that indexed both copies before this
//! slice keeps every vector and indexes nothing again.

use std::collections::HashMap;
use std::fs;

use atlas_lib::domain::dto::{
    DocumentQuery, DocumentSort, FileType, FormatPairBasis, FormatPairState,
};
use atlas_lib::services::{documents, formats, inventory, library};
use rusqlite::Connection;

mod common;
use common::meaning::{LONG, Lib};
use common::wait_until;

/// A library with three books in both formats and the cases beside them
/// (`scripts/generate-fixtures.mjs --pairs`), read and indexed.
fn library_of_pairs() -> Lib {
    Lib::with_fixture(
        &["--files", "4", "--text-kb", "6", "--pairs", "3"],
        &[],
        false,
        &[],
        |ws| ws,
    )
}

/// Each name pair's verdict, by the PDF's name: (verdict, basis).
fn verdicts(c: &Connection) -> HashMap<String, (String, String)> {
    c.prepare(
        "SELECT fp.display_name, bp.verdict, bp.basis
         FROM book_pairs bp
         JOIN file_instances fp ON fp.document_id = bp.pdf_document_id AND fp.file_type = 'pdf'
         WHERE fp.display_path LIKE '%/pairs/%'",
    )
    .unwrap()
    .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn hidden(c: &Connection) -> Vec<String> {
    c.prepare(
        "SELECT f.display_name FROM book_pairs bp
         JOIN file_instances f ON f.document_id = bp.hidden_document_id
         WHERE bp.hidden_document_id IS NOT NULL ORDER BY f.display_name",
    )
    .unwrap()
    .query_map([], |r| r.get(0))
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn file_id(c: &Connection, name: &str) -> (String, String) {
    c.query_row(
        "SELECT id, document_id FROM file_instances WHERE display_name = ?1",
        [name],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

fn embedded(c: &Connection, document: &str) -> bool {
    c.query_row(
        "SELECT EXISTS (SELECT 1 FROM document_embeddings WHERE document_id = ?1)",
        [document],
        |r| r.get(0),
    )
    .unwrap()
}

fn list(
    lib: &Lib,
    text: &str,
    file_type: Option<FileType>,
) -> Vec<atlas_lib::domain::dto::FileInstance> {
    let root = lib.root();
    library::list(
        root.reader(),
        root.root_status(),
        &DocumentQuery {
            text: Some(text.into()),
            source_root_id: None,
            file_type,
            sort: DocumentSort::NameAsc,
            page: 0,
            page_size: 50,
            include_missing: false,
            text_status: None,
        },
    )
    .unwrap()
    .items
}

/// P4-27 to P4-29: the pairs found, one copy indexed, one book everywhere.
#[test]
fn p4_27_a_book_in_both_formats_is_one_book_indexed_once() {
    let lib = library_of_pairs();
    let c = lib.catalog();

    // P4-27: which name pairs are one book.
    let v = verdicts(&c);
    for i in 0..3 {
        let name = format!("paired {i:04}.pdf");
        assert_eq!(v[&name], ("same".into(), "text".into()), "{name}");
    }
    assert_eq!(
        v["Café.PDF"],
        ("same".into(), "text".into()),
        "case and accents fold"
    );
    assert_eq!(v["notes.pdf"].0, "different", "one name, two texts");
    assert_eq!(v["Sampler.pdf"].0, "different", "a chapter isn't the book");
    assert!(!v.contains_key("Atlas.pdf"), "different folders never pair");

    // P4-28: the EPUB of each book is indexed; the PDF waits for nothing.
    assert_eq!(
        hidden(&c),
        [
            "Café.PDF",
            "paired 0000.pdf",
            "paired 0001.pdf",
            "paired 0002.pdf"
        ]
    );
    for name in ["paired 0000", "Café"] {
        let pdf = file_id(
            &c,
            &format!("{name}.{}", if name == "Café" { "PDF" } else { "pdf" }),
        );
        let epub = file_id(
            &c,
            &format!("{}.epub", if name == "Café" { "cafe" } else { name }),
        );
        assert!(
            !embedded(&c, &pdf.1),
            "{name}: the PDF is never sent to be indexed"
        );
        assert!(embedded(&c, &epub.1), "{name}: the EPUB is");
    }
    let ov = atlas_lib::services::embedding::status::overview(&c, false).unwrap();
    assert_eq!(ov.counts.waiting, 0, "the PDF kept out never waits");
    assert_eq!(ov.counts.indexed, ov.counts.total, "{:?}", ov.counts);

    // P4-29: one result for the book, with its other format.
    for page in [
        lib.text_only(&Lib::query("Pairs marker 9000")),
        lib.hybrid(&Lib::query("Pairs marker 9000")),
    ] {
        let hits: Vec<_> = page
            .hits
            .iter()
            .filter(|h| h.display_name.starts_with("paired 0000"))
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "{:?}",
            page.hits
                .iter()
                .map(|h| (&h.display_name, h.why, h.all_words))
                .collect::<Vec<_>>()
        );
        let other = hits[0].other_format.as_ref().expect("the other format");
        assert_ne!(other.file_type, hits[0].file_type);
    }
    // With a type chosen, that format's own result.
    let mut q = Lib::query("Pairs marker 9000");
    q.file_type = Some(FileType::Pdf);
    let pdf_only = lib.text_only(&q);
    assert_eq!(pdf_only.hits[0].display_name, "paired 0000.pdf");

    // The Library lists the book once, with both formats.
    let rows = list(&lib, "paired 0000", None);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].display_name, "paired 0000.epub");
    assert_eq!(
        rows[0].other_format.as_ref().unwrap().display_name,
        "paired 0000.pdf"
    );
    // By type, each stands alone.
    assert_eq!(list(&lib, "paired 0000", Some(FileType::Pdf)).len(), 1);
    // notes are two books, so two rows.
    assert_eq!(list(&lib, "notes", None).len(), 2);

    // The detail panel says what Atlas found.
    let (pdf_file, pdf_doc) = file_id(&c, "paired 0000.pdf");
    let (_, epub_doc) = file_id(&c, "paired 0000.epub");
    let root = lib.root();
    let detail = documents::detail(root.reader(), root.root_status(), &pdf_file).unwrap();
    let pair = detail.formats.unwrap();
    assert_eq!(pair.state, FormatPairState::Same);
    assert_eq!(pair.basis, Some(FormatPairBasis::Text));
    assert_eq!(pair.indexed_as, Some(FileType::Epub));
    assert_eq!(pair.partner.display_name, "paired 0000.epub");

    // Related: asked from either copy, never the book's other copy.
    for doc in [&pdf_doc, &epub_doc] {
        let page = documents::related::run(
            root.reader(),
            root.root_status(),
            root.vectors(),
            &root.catalog().handle(),
            doc,
            20,
        )
        .unwrap();
        assert!(
            page.hits
                .iter()
                .all(|h| h.document_id != pdf_doc && h.document_id != epub_doc),
            "{:?}",
            page.hits
                .iter()
                .map(|h| &h.display_name)
                .collect::<Vec<_>>()
        );
        let hidden_docs: Vec<String> = hidden(&c).iter().map(|n| file_id(&c, n).1).collect();
        assert!(
            page.hits
                .iter()
                .all(|h| !hidden_docs.contains(&h.document_id))
        );
    }
}

/// A library that indexed both copies before this slice (the owner's,
/// 2026-11-13): after the upgrade every vector is kept, nothing is indexed
/// again, and each book is shown once.
#[test]
fn p4_28_an_upgraded_library_indexes_nothing_again() {
    let mut lib = library_of_pairs();
    // As before Slice 4.5.1: no pairs, every copy indexed. (The folder
    // stays marked as paired meanwhile: a meaning job pairs a folder that
    // isn't before it starts, which is the point of this slice.)
    let c = lib.catalog();
    c.execute_batch("DELETE FROM book_pairs; DELETE FROM format_pairs;")
        .unwrap();
    drop(c);
    atlas_lib::services::embedding::triggers::changed(lib.root()).unwrap();
    assert!(wait_until(LONG, || lib.indexed()), "every copy indexed");
    let c = lib.catalog();
    let (_, pdf_doc) = file_id(&c, "paired 0001.pdf");
    assert!(embedded(&c, &pdf_doc), "the PDF was indexed too");
    let sent = lib.h.script.answered().len();
    c.execute("UPDATE source_roots SET formats_version = 0", [])
        .unwrap();
    drop(c);

    // The upgrade: the library opens with this build.
    lib.ws = lib.env.other_workspace();
    let mut ws = lib.env.workspace_with_embedding(&lib.h);
    ws.startup();
    lib.ws = ws;
    assert!(
        wait_until(LONG, || hidden(&lib.catalog()).len() == 4),
        "paired after the upgrade: {:?}; jobs {:?}",
        hidden(&lib.catalog()),
        lib.catalog()
            .prepare("SELECT kind, state, last_error FROM jobs ORDER BY rowid")
            .unwrap()
            .query_map([], |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?
            )))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
    );
    // Nothing waits, so the index opens when search first asks for it.
    assert!(
        wait_until(LONG, || {
            lib.hybrid(&Lib::query("Pairs marker 9001"));
            lib.indexed()
        }),
        "the index opened, with nothing left to index"
    );
    let c = lib.catalog();
    let behind: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM source_roots WHERE formats_version < 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(behind, 0);
    assert_eq!(
        lib.h.script.answered().len(),
        sent,
        "nothing was sent to be indexed again"
    );
    assert!(embedded(&c, &pdf_doc), "the PDF's vectors are kept");
    // Both were indexed, so the EPUB is used.
    assert_eq!(
        hidden(&c),
        [
            "Café.PDF",
            "paired 0000.pdf",
            "paired 0001.pdf",
            "paired 0002.pdf"
        ]
    );
    let page = lib.hybrid(&Lib::query("Pairs marker 9001"));
    let hits: Vec<_> = page
        .hits
        .iter()
        .filter(|h| h.display_name.starts_with("paired 0001"))
        .collect();
    assert_eq!(hits.len(), 1, "one result, though both copies have vectors");
}

/// P4-30: the owner's choice is kept, and a copy whose partner is gone is
/// shown and indexed again.
#[test]
fn p4_30_the_owners_choice_is_kept_and_a_lone_copy_comes_back() {
    let lib = library_of_pairs();
    let c = lib.catalog();
    let (pdf_file, pdf_doc) = file_id(&c, "paired 0002.pdf");
    let (_, notes_doc) = file_id(&c, "notes.pdf");
    let (notes_file, _) = file_id(&c, "notes.epub");
    drop(c);
    let root = lib.root();

    // Not the same book: both shown, and the PDF is indexed.
    formats::set_same_book(root, &pdf_file, false).unwrap();
    let detail = documents::detail(root.reader(), root.root_status(), &pdf_file).unwrap();
    let pair = detail.formats.unwrap();
    assert_eq!(
        (pair.state, pair.basis),
        (FormatPairState::Different, Some(FormatPairBasis::Owner))
    );
    assert!(
        wait_until(LONG, || embedded(&lib.catalog(), &pdf_doc)),
        "the PDF indexed"
    );
    assert_eq!(list(&lib, "paired 0002", None).len(), 2);

    // Kept over a rescan and a re-read.
    let scan = inventory::control::rescan(root, &detail.file.source_root_id).unwrap();
    assert!(wait_until(LONG, || {
        lib.catalog()
            .query_row("SELECT state FROM jobs WHERE id = ?1", [&scan], |r| {
                r.get::<_, String>(0)
            })
            .is_ok_and(|s| s == "completed")
    }));
    lib.catalog()
        .execute(
            "UPDATE documents SET updated_at = 'later' WHERE id = ?1",
            [&pdf_doc],
        )
        .unwrap();
    let still: String = lib
        .catalog()
        .query_row(
            "SELECT verdict FROM book_pairs WHERE pdf_document_id = ?1",
            [&pdf_doc],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(still, "different");

    // Same book, for a pair the check rejected.
    formats::set_same_book(root, &notes_file, true).unwrap();
    assert!(hidden(&lib.catalog()).contains(&"notes.pdf".to_owned()));
    let _ = notes_doc;

    // The EPUB of a book deleted: its PDF is a book of its own again.
    let books = lib.env.base.join("Books/pairs");
    fs::remove_file(books.join("paired 0001.epub")).unwrap();
    let scan = inventory::control::rescan(root, &detail.file.source_root_id).unwrap();
    assert!(wait_until(LONG, || {
        inventory::control::overview(root).is_ok_and(|o| {
            o.active.is_none()
                && lib
                    .catalog()
                    .query_row("SELECT state FROM jobs WHERE id = ?1", [&scan], |r| {
                        r.get::<_, String>(0)
                    })
                    .is_ok_and(|s| s == "completed")
        })
    }));
    assert!(!hidden(&lib.catalog()).contains(&"paired 0001.pdf".to_owned()));
    let (_, lone) = file_id(&lib.catalog(), "paired 0001.pdf");
    assert!(
        wait_until(LONG, || embedded(&lib.catalog(), &lone)),
        "the lone PDF indexed"
    );
}

/// What the text check costs on book-sized texts (Slice 4.5.1): each pair's
/// check holds the catalog writer, so its time is how long other writes
/// wait. `just bench-formats` (release); numbers only.
#[test]
#[ignore = "a measurement: just bench-formats"]
fn measure_the_text_check() {
    use common::{Env, ExtractionHarness, fast_extraction, worker};
    use std::process::Command;
    use std::time::Instant;

    let env = Env::new();
    let books = env.base.join("Books");
    let out = Command::new("node")
        .arg(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../scripts/generate-fixtures.mjs"),
        )
        .arg(&books)
        .args(["--files", "2", "--text-kb", "400", "--pairs", "20"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = env.workspace_with_extraction(&h);
    ws.choose(&env.base.join("AtlasData")).unwrap();
    let root = ws.require_open().unwrap();
    let added = atlas_lib::services::sources::add(root, &books).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    assert!(wait_until(std::time::Duration::from_secs(600), || {
        root.catalog()
            .call(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM source_roots WHERE formats_version >= 1",
                    [],
                    |r| r.get::<_, i64>(0),
                )
            })
            .is_ok_and(|n| n.is_ok_and(|n| n == 1))
            && hidden(&lib_catalog(&env)).len() >= 20
    }));
    // Check every pair again, one at a time, timing each.
    let root_id = added.id.clone();
    let pairs: Vec<(String, String)> = root
        .catalog()
        .call(move |c| {
            c.execute("DELETE FROM book_pairs", []).unwrap();
            formats::db::pending_checks(c, &root_id, 1_000).unwrap()
        })
        .unwrap();
    let mut times = Vec::new();
    for (p, e) in pairs.clone() {
        let started = Instant::now();
        root.catalog()
            .call(move |c| formats::db::check_pair(c, &p, &e, "t").unwrap())
            .unwrap();
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    let words: i64 = root
        .catalog()
        .call(|c| {
            c.query_row(
                "SELECT COALESCE(SUM(char_count), 0) / COUNT(*) FROM documents WHERE state = 'done'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap()
        .unwrap();
    let p = |q: f64| times[((times.len() - 1) as f64 * q).round() as usize];
    println!(
        "text check: {} pairs, about {} characters a book; per pair p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        times.len(),
        words,
        p(0.5),
        p(0.95),
        times.last().unwrap()
    );
}

fn lib_catalog(env: &common::Env) -> Connection {
    Connection::open(env.base.join("AtlasData/library/catalog.sqlite")).unwrap()
}
