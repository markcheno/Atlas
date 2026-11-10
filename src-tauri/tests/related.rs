//! Slice 2.5: related documents (phase2-spec §7.5).
//!
//! - P2-12: a document's related documents never include itself or its
//!   copies (one document), nor a document with the same text in different
//!   bytes; a document without embeddings says so.
//! - Unrelated documents fall under the similarity floor; a document with
//!   no available file is left out; nothing is sent to the AI service; the
//!   states (off, loading, no text) are reported, never errors.

use atlas_lib::domain::dto::{Availability, RelatedPage, RelatedStatus};
use atlas_lib::services::documents::related;
use atlas_lib::services::embedding::models;

mod common;
use common::meaning::{BREAD, LONG, Lib, SEA, SKY};
use common::wait_until;

/// Shares the sky sentence's subject and most of its words, not its text.
const NEAR_SKY: &str =
    "Distant comets and glowing nebulae drifted past the starship on its long voyage.";

fn related(lib: &Lib, document: &str, limit: u32) -> RelatedPage {
    let root = lib.root();
    related::run(
        root.reader(),
        root.root_status(),
        root.vectors(),
        &root.catalog().handle(),
        document,
        limit,
    )
    .unwrap()
}

fn ids(page: &RelatedPage) -> Vec<&str> {
    page.hits.iter().map(|h| h.document_id.as_str()).collect()
}

fn sql(lib: &Lib, statement: &str, doc: &str) {
    let (statement, doc) = (statement.to_owned(), doc.to_owned());
    lib.root()
        .catalog()
        .call(move |c| c.execute(&statement, [&doc]).unwrap())
        .unwrap();
}

#[test]
fn p2_12_related_documents_leave_out_the_document_its_copies_and_its_text() {
    // The sky document has copies; another document has its text in other
    // bytes; one is close in meaning; two are on other subjects.
    let lib = Lib::with_texts(&[SKY, NEAR_SKY, SKY, BREAD, SEA], true);
    let sky = lib.docs[0].0.clone();
    let (near, same_text) = (lib.docs[1].0.as_str(), lib.docs[2].0.as_str());
    let copies: i64 = lib
        .catalog()
        .query_row(
            "SELECT COUNT(*) FROM file_instances WHERE document_id = ?1",
            [&sky],
            |r| r.get(0),
        )
        .unwrap();
    assert!(copies >= 2, "the fixture gives the first document copies");

    let calls = lib.h.script.calls();
    let page = related(&lib, &sky, 10);
    assert_eq!(page.status, RelatedStatus::Ready);
    let found = ids(&page);
    assert_eq!(
        found.first(),
        Some(&near),
        "the close document first: {found:?}"
    );
    assert!(!found.contains(&sky.as_str()), "never itself or its copies");
    assert!(
        !found.contains(&same_text),
        "never the same text in other bytes"
    );
    for subject in [&lib.docs[3].0, &lib.docs[4].0] {
        assert!(
            !found.contains(&subject.as_str()),
            "other subjects fall under the floor"
        );
    }
    let first = &page.hits[0];
    assert_eq!(first.availability, Availability::Available);
    assert!(first.location.is_some(), "the best matching place");
    assert_eq!(
        lib.h.script.calls(),
        calls,
        "nothing was sent to the AI service"
    );

    // The same text is hidden both ways; the close one lists the sky.
    assert!(!ids(&related(&lib, same_text, 10)).contains(&sky.as_str()));
    let back = related(&lib, near, 10);
    assert!(ids(&back).contains(&sky.as_str()) || ids(&back).contains(&same_text));

    // Limits are honoured and clamped.
    assert!(related(&lib, &sky, 1).hits.len() <= 1);
    assert!(related(&lib, &sky, 0).hits.len() <= 1);
    assert!(related(&lib, &sky, 1_000).hits.len() <= related::MAX_LIMIT as usize);

    // A document with no available file drops out at once (the cache keeps
    // candidates, never files).
    sql(
        &lib,
        "UPDATE file_instances SET availability = 'missing' WHERE document_id = ?1",
        near,
    );
    assert!(!ids(&related(&lib, &sky, 10)).contains(&near));

    // A document whose embedding failed has no related documents yet.
    sql(
        &lib,
        "UPDATE document_embeddings SET status = 'failed' WHERE document_id = ?1",
        &lib.docs[3].0,
    );
    let page = related(&lib, &lib.docs[3].0, 10);
    assert_eq!(page.status, RelatedStatus::NotIndexed);
    assert!(page.hits.is_empty());
    let page = related(&lib, "no-such-document", 10);
    assert_eq!(page.status, RelatedStatus::NotIndexed);
}

#[test]
fn related_documents_report_their_state_and_never_fail() {
    let lib = Lib::with_texts(&[SKY, NEAR_SKY, ""], false);
    let (sky, empty) = (lib.docs[0].0.clone(), lib.docs[2].0.clone());

    // No text to compare.
    let status: String = lib
        .catalog()
        .query_row(
            "SELECT status FROM document_embeddings WHERE document_id = ?1",
            [&empty],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "empty");
    assert_eq!(related(&lib, &empty, 10).status, RelatedStatus::NoText);

    // The index closed: loading, then ready, never waiting for it.
    let model = models::serving(&lib.catalog()).unwrap().unwrap();
    lib.root().vectors().forget(&model.id);
    let page = related(&lib, &sky, 10);
    assert_eq!(page.status, RelatedStatus::Loading);
    assert!(page.hits.is_empty());
    assert!(
        wait_until(LONG, || related(&lib, &sky, 10).status
            == RelatedStatus::Ready),
        "the index opened in the background"
    );
    assert_eq!(
        ids(&related(&lib, &sky, 10)).first(),
        Some(&lib.docs[1].0.as_str())
    );

    // No embedding model: nothing to show.
    sql(
        &lib,
        "DELETE FROM model_roles WHERE role = 'embedding' AND ?1 <> ''",
        "x",
    );
    let page = related(&lib, &sky, 10);
    assert_eq!(page.status, RelatedStatus::Off);
    assert!(page.hits.is_empty());
}
