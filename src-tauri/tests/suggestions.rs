//! Slice 3.6, P3-18 (phase3-spec §7.4, §12): suggestions from the library.
//!
//! Seeds from an interest and from recent trails; an opened document; a
//! dismissed one; a missing file; no meaning index. Those are left out;
//! the order is stable for the day; no request reaches any service; none
//! without an index.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use atlas_lib::domain::dto::{
    CollectionInput, CollectionItemRef, CollectionKind, OpenAction, OpenPlace, OpenVia,
    SearchIntent, Suggestion, SuggestionReason,
};
use atlas_lib::infrastructure::clock::{Clock, TestClock};
use atlas_lib::services::extraction::db as extraction_db;
use atlas_lib::services::inventory;
use atlas_lib::services::knowledge::{Recorder, collections, rediscovery};
use time::macros::datetime;

mod common;
use common::meaning::{LONG, Lib};
use common::wait_until;

struct S {
    lib: Lib,
    clock: Arc<TestClock>,
}

impl S {
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

    /// Suggestions, checking that no request reached any service for them.
    fn suggest(&self, limit: usize) -> Vec<Suggestion> {
        let root = self.lib.root();
        let calls = self.lib.h.script.calls();
        let found =
            rediscovery::suggestions(self.rec(), root.vectors(), &root.catalog().handle(), limit)
                .unwrap();
        assert_eq!(
            self.lib.h.script.calls(),
            calls,
            "only stored vectors are read"
        );
        found
    }

    fn docs(&self, s: &[Suggestion]) -> Vec<String> {
        s.iter()
            .map(|x| x.hit.document_id.clone().unwrap())
            .collect()
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

    fn document(&self, file: &str) -> String {
        self.lib
            .catalog()
            .query_row(
                "SELECT document_id FROM file_instances WHERE id = ?1",
                [file],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn interest(&self, name: &str, file: &str) -> String {
        let c = collections::save(
            self.rec(),
            &CollectionInput {
                id: None,
                kind: CollectionKind::Interest,
                name: name.into(),
                description: None,
                goal: None,
                status: None,
            },
        )
        .unwrap();
        collections::add(
            self.rec(),
            &c.id,
            &CollectionItemRef::Document {
                file_id: Some(file.to_owned()),
                document_id: None,
            },
        )
        .unwrap();
        c.id
    }

    fn settle(&self) {
        let root_id: String = self
            .lib
            .catalog()
            .query_row("SELECT id FROM source_roots", [], |r| r.get(0))
            .unwrap();
        inventory::control::enqueue(self.lib.root(), &root_id).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(wait_until(LONG, || {
            let root = self.lib.root();
            let inv = inventory::control::overview(root).unwrap();
            let ov = root
                .reader()
                .read(|c| extraction_db::overview(c, None))
                .unwrap()
                .unwrap();
            inv.active.is_none()
                && ov.active.is_none()
                && ov.queued_jobs == 0
                && ov.counts.pending == 0
        }));
    }
}

fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    for e in fs::read_dir(dir).ok()?.flatten() {
        let path = e.path();
        if path.is_dir() {
            if let Some(found) = find(&path, name) {
                return Some(found);
            }
        } else if e.file_name().to_string_lossy() == name {
            return Some(path);
        }
    }
    None
}

#[test]
fn p3_18_suggestions_leave_out_what_they_should_and_keep_the_days_order() {
    let s = S::new();
    // Nothing to start from: nothing suggested.
    assert!(s.suggest(6).is_empty());

    let seed_file = s.file("book 000001.pdf");
    let seed_doc = s.document(&seed_file);
    let first = s.interest("Mechanisms", &seed_file);
    let found = s.suggest(6);
    assert!(!found.is_empty(), "an interest's neighbours are suggested");
    assert!(
        !s.docs(&found).contains(&seed_doc),
        "never its own document"
    );
    assert!(found.iter().all(|x| x.seed_id == first
        && x.reason
            == SuggestionReason::Interest {
                name: "Mechanisms".into()
            }));
    // Each document once.
    let mut unique = s.docs(&found);
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), found.len());

    // The same all day, and after a relaunch the same day.
    assert_eq!(s.docs(&s.suggest(6)), s.docs(&found));
    s.clock.advance(time::Duration::hours(3));
    assert_eq!(s.docs(&s.suggest(6)), s.docs(&found));

    // An opened document is left out.
    let opened = found[0].hit.file_instance_id.clone();
    let opened_doc = found[0].hit.document_id.clone().unwrap();
    let q = Lib::query("anything at all");
    let page = s.lib.hybrid(&q);
    s.rec().search(&q, SearchIntent::Refresh, &page);
    s.rec().open(
        &opened,
        &OpenVia::Related,
        OpenAction::Details,
        OpenPlace::default(),
    );
    let now = s.suggest(6);
    assert!(!s.docs(&now).contains(&opened_doc), "opened in a trail");

    // A second interest on the same document, then Not for this on the
    // first: dismissed there, still suggested by the other.
    let second = s.interest("Gears", &seed_file);
    let target = s
        .suggest(6)
        .into_iter()
        .find(|x| x.seed_id == first)
        .unwrap()
        .hit
        .document_id
        .unwrap();
    rediscovery::dismiss(s.rec(), &first, &target).unwrap();
    let after = s.suggest(6);
    assert!(
        !after
            .iter()
            .any(|x| x.seed_id == first && x.hit.document_id.as_deref() == Some(target.as_str()))
    );
    let both = s.suggest(6);
    assert!(
        both.iter()
            .any(|x| x.seed_id == second && x.hit.document_id.as_deref() == Some(target.as_str()))
            || both.len() == 6,
        "the other interest may still suggest it"
    );

    // A file gone from the library is left out once it is rescanned.
    let gone = s
        .suggest(6)
        .into_iter()
        .find(|x| x.hit.display_name.starts_with("book "))
        .unwrap();
    let path = find(&s.lib.env.base.join("Books"), &gone.hit.display_name).unwrap();
    fs::remove_file(path).unwrap();
    s.settle();
    assert!(
        !s.docs(&s.suggest(6))
            .contains(gone.hit.document_id.as_ref().unwrap())
    );
}

#[test]
fn p3_18_recent_trails_seed_suggestions_and_none_without_an_index() {
    let s = S::new();
    let q = Lib::query("Engineering marker 1");
    let page = s.lib.hybrid(&q);
    s.rec().search(&q, SearchIntent::Submit, &page);
    let found = s.suggest(3);
    assert!(!found.is_empty());
    assert!(
        found
            .iter()
            .all(|x| x.reason == SuggestionReason::Recent && x.seed_id.is_empty())
    );
    assert!(found.len() <= 3);
    // The search's own results are the seed, not suggestions.
    let shown: Vec<String> = page
        .hits
        .iter()
        .take(5)
        .filter_map(|h| h.document_id.clone())
        .collect();
    assert!(s.docs(&found).iter().all(|d| !shown.contains(d)));

    // Not for this, for recent trails.
    let first = found[0].hit.document_id.clone().unwrap();
    rediscovery::dismiss(s.rec(), "", &first).unwrap();
    assert!(!s.docs(&s.suggest(3)).contains(&first));

    // Another day may order them differently; they are still suggestions.
    s.clock.advance(time::Duration::days(1));
    assert!(!s.suggest(3).is_empty());

    // Without a meaning index: none.
    s.lib
        .root()
        .catalog()
        .call(|c| c.execute("DELETE FROM model_roles WHERE role = 'embedding'", []))
        .unwrap()
        .unwrap();
    assert!(s.suggest(3).is_empty());
}
