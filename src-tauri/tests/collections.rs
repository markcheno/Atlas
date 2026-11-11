//! Slice 3.6, P3-17 (phase3-spec §5.4, §12): interests and projects with
//! items of each kind; deleting an item; finishing and archiving; deleting
//! a collection. Items leave their collections; a deleted collection never
//! deletes its items.

use std::sync::Arc;

use atlas_lib::domain::dto::{
    CollectionInput, CollectionItemRef, CollectionKind, CollectionStatus, FilePlace, FindingSource,
    SaveTarget, SavedKind, SearchIntent,
};
use atlas_lib::error::ErrorCode;
use atlas_lib::infrastructure::clock::{Clock, TestClock};
use atlas_lib::services::knowledge::{Recorder, collections, saved};
use rusqlite::Connection;
use time::macros::datetime;

mod common;
use common::code;
use common::meaning::Lib;

struct C {
    lib: Lib,
    clock: Arc<TestClock>,
}

impl C {
    fn new() -> Self {
        let clock = Arc::new(TestClock::at(datetime!(2026-11-11 09:00 UTC)));
        let c = clock.clone();
        let lib = Lib::custom(&[], false, 0, &[], move |ws| {
            ws.with_clock(c as Arc<dyn Clock>)
        });
        Self { lib, clock }
    }

    fn rec(&self) -> &Arc<Recorder> {
        self.lib.root().recorder()
    }

    fn later(&self) {
        self.clock.advance(time::Duration::minutes(1));
    }

    fn file(&self, name: &str) -> String {
        self.lib
            .catalog()
            .query_row(
                "SELECT id FROM file_instances WHERE display_name = ?1",
                [name],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn input(&self, kind: CollectionKind, name: &str) -> CollectionInput {
        CollectionInput {
            id: None,
            kind,
            name: name.into(),
            description: None,
            goal: None,
            status: None,
        }
    }

    fn count(&self, sql: &str) -> i64 {
        Connection::open(self.lib.env.base.join("AtlasData/library/knowledge.sqlite"))
            .unwrap()
            .query_row(sql, [], |r| r.get(0))
            .unwrap()
    }
}

#[test]
fn p3_17_items_of_each_kind_leave_their_collections_never_the_other_way() {
    let c = C::new();
    // A trail, a bookmark, a finding and a document.
    let q = Lib::query("Engineering marker 1");
    let page = c.lib.hybrid(&q);
    c.rec().search(&q, SearchIntent::Submit, &page);
    let trail = c.rec().status().current_trail_id.unwrap();
    let book = c.file("book 000001.pdf");
    let bookmark = saved::save_bookmark(
        c.rec(),
        &SaveTarget::File {
            file_id: book.clone(),
            place: FilePlace::Result,
        },
    )
    .unwrap();
    let finding = saved::save_finding(
        c.rec(),
        &FindingSource::Passage {
            target: SaveTarget::File {
                file_id: book,
                place: FilePlace::Result,
            },
        },
    )
    .unwrap();
    let other = c.file("book 000002.pdf");

    let interest = collections::save(
        c.rec(),
        &CollectionInput {
            description: Some("  Gears and drives ".into()),
            ..c.input(CollectionKind::Interest, " Mechanisms ")
        },
    )
    .unwrap();
    assert_eq!(
        (interest.name.as_str(), interest.description.as_deref()),
        ("Mechanisms", Some("Gears and drives"))
    );
    assert_eq!(interest.status, CollectionStatus::Active);
    let items = [
        CollectionItemRef::Trail { id: trail.clone() },
        CollectionItemRef::Bookmark {
            id: bookmark.id.clone(),
        },
        CollectionItemRef::Finding {
            id: finding.id.clone(),
        },
        CollectionItemRef::Document {
            file_id: Some(other.clone()),
            document_id: None,
        },
    ];
    for item in &items {
        c.later();
        collections::add(c.rec(), &interest.id, item).unwrap();
    }
    // Adding again changes nothing.
    let again = collections::add(c.rec(), &interest.id, &items[0]).unwrap();
    assert_eq!(
        (
            again.counts.trails,
            again.counts.bookmarks,
            again.counts.findings,
            again.counts.documents
        ),
        (1, 1, 1, 1)
    );
    let detail = collections::get(c.rec(), &interest.id).unwrap();
    assert_eq!(detail.trails[0].id, trail);
    assert_eq!(detail.bookmarks[0].id, bookmark.id);
    assert_eq!(detail.findings[0].id, finding.id);
    assert!(detail.documents[0].title.is_some());
    assert_eq!(
        detail.documents[0].resolved.state,
        atlas_lib::domain::dto::AnchorState::Found
    );
    let member = collections::membership(c.rec(), &items[1]).unwrap();
    assert_eq!(member.collection_ids, vec![interest.id.clone()]);

    // A project holds the same items; taking one out leaves the item.
    let project = collections::save(
        c.rec(),
        &CollectionInput {
            goal: Some("A chapter on drives".into()),
            ..c.input(CollectionKind::Project, "Thesis")
        },
    )
    .unwrap();
    for item in &items {
        collections::add(c.rec(), &project.id, item).unwrap();
    }
    let less = collections::remove(c.rec(), &project.id, &items[1]).unwrap();
    assert_eq!(less.counts.bookmarks, 0);
    assert_eq!(
        c.count("SELECT count(*) FROM bookmarks"),
        1,
        "the bookmark stays"
    );

    // Deleting an item takes it out of every collection.
    c.rec().delete(&trail).unwrap();
    saved::delete(c.rec(), SavedKind::Finding, &finding.id).unwrap();
    let after = collections::get(c.rec(), &interest.id).unwrap();
    assert!(after.trails.is_empty() && after.findings.is_empty());
    assert_eq!(after.collection.counts.trails, 0);
    assert_eq!(after.bookmarks.len(), 1);
    assert_eq!(
        c.count("SELECT count(*) FROM collection_items WHERE item_kind IN ('trail', 'finding')"),
        0
    );
    // Adding something no longer there is refused.
    assert_eq!(
        code(collections::add(c.rec(), &interest.id, &items[0])),
        ErrorCode::TrailNotFound
    );
    assert_eq!(
        code(collections::add(c.rec(), "gone", &items[1])),
        ErrorCode::CollectionNotFound
    );

    // Deleting a collection never deletes what it held.
    collections::delete(c.rec(), &interest.id).unwrap();
    assert_eq!(c.count("SELECT count(*) FROM bookmarks"), 1);
    assert_eq!(
        c.count(&format!(
            "SELECT count(*) FROM collection_items WHERE collection_id = '{}'",
            interest.id
        )),
        0
    );
    assert_eq!(
        code(collections::get(c.rec(), &interest.id)),
        ErrorCode::CollectionNotFound
    );
}

#[test]
fn p3_17_finishing_archiving_and_what_each_kind_may_hold() {
    let c = C::new();
    let project = collections::save(c.rec(), &c.input(CollectionKind::Project, "Thesis")).unwrap();
    let interest =
        collections::save(c.rec(), &c.input(CollectionKind::Interest, "Stoicism")).unwrap();

    // A project can be finished; an interest can't, nor have a goal.
    let finished = collections::save(
        c.rec(),
        &CollectionInput {
            id: Some(project.id.clone()),
            status: Some(CollectionStatus::Finished),
            ..c.input(CollectionKind::Project, "Thesis")
        },
    )
    .unwrap();
    assert_eq!(finished.status, CollectionStatus::Finished);
    for bad in [
        CollectionInput {
            id: Some(interest.id.clone()),
            status: Some(CollectionStatus::Finished),
            ..c.input(CollectionKind::Interest, "Stoicism")
        },
        CollectionInput {
            goal: Some("A goal".into()),
            ..c.input(CollectionKind::Interest, "Epicureans")
        },
        c.input(CollectionKind::Interest, "   "),
    ] {
        assert_eq!(
            code(collections::save(c.rec(), &bad)),
            ErrorCode::InvalidCollection
        );
    }
    assert_eq!(
        code(collections::save(
            c.rec(),
            &c.input(CollectionKind::Interest, &"n".repeat(201))
        )),
        ErrorCode::TextTooLong
    );
    // A collection keeps its kind.
    let kept = collections::save(
        c.rec(),
        &CollectionInput {
            id: Some(interest.id.clone()),
            ..c.input(CollectionKind::Project, "Stoics")
        },
    )
    .unwrap();
    assert_eq!(
        (kept.kind, kept.name.as_str()),
        (CollectionKind::Interest, "Stoics")
    );

    // Archived and finished ones aren't active; unarchiving brings it back.
    collections::save(
        c.rec(),
        &CollectionInput {
            id: Some(interest.id.clone()),
            status: Some(CollectionStatus::Archived),
            ..c.input(CollectionKind::Interest, "Stoics")
        },
    )
    .unwrap();
    assert!(
        collections::list(c.rec(), Some(CollectionStatus::Active))
            .unwrap()
            .is_empty()
    );
    assert_eq!(collections::list(c.rec(), None).unwrap().len(), 2);
    collections::save(
        c.rec(),
        &CollectionInput {
            id: Some(interest.id.clone()),
            status: Some(CollectionStatus::Active),
            ..c.input(CollectionKind::Interest, "Stoics")
        },
    )
    .unwrap();
    let active = collections::list(c.rec(), Some(CollectionStatus::Active)).unwrap();
    assert_eq!(
        active.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
        [interest.id]
    );
}
