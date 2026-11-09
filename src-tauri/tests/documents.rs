//! Slice 1.5: document details for the panel (phase1-spec §5.3): title,
//! authors and pages from the read document, the file's text status, and
//! the other copies of its content.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use atlas_lib::domain::dto::{Availability, ExtractionState};
use atlas_lib::error::ErrorCode;
use atlas_lib::services::extraction::db;
use atlas_lib::services::workspace::Workspace;
use atlas_lib::services::{documents, inventory, sources};

mod common;
use common::{
    Env, ExtractionHarness, InventoryHarness, code, fast_config, fast_extraction, wait_until,
    worker,
};

const LONG: Duration = Duration::from_secs(180);

/// A small generated text fixture: PDFs carry /Title and /Author, EPUBs
/// dc:title and dc:creator.
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

fn settle(ws: &Workspace) {
    let root = ws.require_open().unwrap();
    assert!(wait_until(LONG, || {
        let inv = inventory::control::overview(root).unwrap();
        let ov = root
            .reader()
            .read(|c| db::overview(c, None))
            .unwrap()
            .unwrap();
        inv.active.is_none() && ov.active.is_none() && ov.counts.pending == 0
    }));
}

fn file_id(ws: &Workspace, path: &Path) -> String {
    let display = dunce::canonicalize(path).unwrap();
    let display = display.to_string_lossy().into_owned();
    ws.require_open()
        .unwrap()
        .reader()
        .read(move |c| {
            c.query_row(
                "SELECT id FROM file_instances WHERE display_path = ?1",
                [display],
                |r| r.get::<_, String>(0),
            )
        })
        .unwrap()
        .unwrap()
}

#[test]
fn a_read_pdf_shows_its_metadata_pages_and_other_copies() {
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let env = Env::new();
    let mut ws = env.workspace_with_extraction(&h);
    ws.choose(&env.base.join("AtlasData")).unwrap();
    let books = env.base.join("Books");
    fixture(&books);
    // book 000001 is Engineering/level-0-1/book 000001.pdf (author Bo Okafor).
    let original: PathBuf = books.join("Engineering/level-0-1/book 000001.pdf");
    let copy = books.join("Engineering/copy of book 000001.pdf");
    fs::copy(&original, &copy).unwrap();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &books).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    settle(&ws);

    let root = ws.require_open().unwrap();
    let d = documents::detail(root.reader(), root.root_status(), &file_id(&ws, &original)).unwrap();
    assert_eq!(d.file.display_name, "book 000001.pdf");
    assert_eq!(d.file.extraction.state, ExtractionState::Done);
    assert!(d.document_id.is_some());
    assert_eq!(d.title.as_deref(), Some("Engineering book 000001"));
    assert_eq!(d.authors, vec!["Bo Okafor".to_owned()]);
    assert!(d.page_count.unwrap() >= 2);
    assert!(d.char_count.unwrap() > 1000);
    assert_eq!(d.also_at.len(), 1);
    assert_eq!(d.also_at[0].display_name, "copy of book 000001.pdf");
    assert_eq!(d.also_at[0].document_id, d.document_id);

    // The copy goes missing: still listed, marked missing.
    fs::remove_file(&copy).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    settle(&ws);
    let d = documents::detail(root.reader(), root.root_status(), &file_id(&ws, &original)).unwrap();
    assert_eq!(d.also_at.len(), 1);
    assert_eq!(d.also_at[0].availability, Availability::Missing);

    // An EPUB's title, creator and language come from its OPF.
    let epub = documents::detail(
        root.reader(),
        root.root_status(),
        &file_id(&ws, &books.join("Astronomy/Café crème book 000000.epub")),
    )
    .unwrap();
    assert_eq!(
        epub.title.as_deref(),
        Some("Astronomy Café crème book 000000")
    );
    assert_eq!(epub.authors, vec!["Ada Lin".to_owned()]);
    assert_eq!(epub.language.as_deref(), Some("en"));

    assert_eq!(
        code(documents::detail(
            root.reader(),
            root.root_status(),
            "no-such-file"
        )),
        ErrorCode::FileNotFound
    );
}

#[test]
fn an_unread_file_has_details_but_no_document() {
    let h = InventoryHarness::new(fast_config());
    let env = Env::new();
    // Cataloging only: nothing reads text.
    let mut ws = env.workspace_with_inventory(h.deps.clone());
    ws.choose(&env.base.join("AtlasData")).unwrap();
    let books = env.base.join("Books");
    fixture(&books);
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &books).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    assert!(wait_until(LONG, || inventory::control::overview(root)
        .unwrap()
        .active
        .is_none()));
    let d = documents::detail(
        root.reader(),
        root.root_status(),
        &file_id(&ws, &books.join("Engineering/level-0-1/book 000001.pdf")),
    )
    .unwrap();
    assert_eq!(d.file.extraction.state, ExtractionState::Pending);
    assert!(d.document_id.is_none() && d.title.is_none() && d.also_at.is_empty());
}
