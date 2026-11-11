//! Slice 3.5, P3-15 (phase3-spec §7.3, §12): bookmarks and findings saved
//! from a result, a citation, a page, the whole document, a trail step and
//! an answer; the same place twice; a note at the limit and over it.
//!
//! Anchors are built in Rust from the catalog and what the screen showed;
//! one bookmark per place; the note is capped.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use atlas_lib::domain::dto::{
    AnchorState, FilePlace, FindingKind, FindingSource, SaveTarget, SavedItem, SavedKind,
    SavedQuery, SearchIntent, SearchPage, StepPayload, TrailQuery,
};
use atlas_lib::error::ErrorCode;
use atlas_lib::infrastructure::clock::{Clock, TestClock};
use atlas_lib::services::extraction::db as extraction_db;
use atlas_lib::services::inventory;
use atlas_lib::services::knowledge::anchor::to_hex;
use atlas_lib::services::knowledge::{Recorder, saved};
use rusqlite::Connection;
use time::macros::datetime;

mod common;
use common::chat::{Chat, sse, streamed};
use common::meaning::{LONG, Lib};
use common::{code, wait_until};

struct Saved {
    lib: Lib,
    clock: Arc<TestClock>,
    chat: Chat,
}

impl Saved {
    fn new() -> Self {
        let clock = Arc::new(TestClock::at(datetime!(2026-11-11 09:00 UTC)));
        let c = clock.clone();
        let lib = Lib::custom(&[], false, 0, &[], move |ws| {
            ws.with_clock(c as Arc<dyn Clock>)
        });
        let chat = Chat::new(&lib);
        Self { lib, clock, chat }
    }

    fn rec(&self) -> &Arc<Recorder> {
        self.lib.root().recorder()
    }

    fn later(&self) {
        self.clock.advance(time::Duration::minutes(1));
    }

    fn search(&self, words: &str) -> SearchPage {
        let q = Lib::query(words);
        let page = self.lib.hybrid(&q);
        self.rec().search(&q, SearchIntent::Submit, &page);
        page
    }

    fn file(&self, page: &SearchPage, name: &str) -> (String, Option<i64>) {
        let hit = page.hits.iter().find(|h| h.display_name == name).unwrap();
        (hit.file_instance_id.clone(), hit.passage)
    }

    fn file_id(&self, name: &str) -> String {
        self.lib
            .catalog()
            .query_row(
                "SELECT id FROM file_instances WHERE display_name = ?1 AND availability = 'available'",
                [name],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn ordinal(&self, passage: i64) -> u32 {
        self.lib
            .catalog()
            .query_row(
                "SELECT ordinal FROM passages WHERE id = ?1",
                [passage],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn store(&self) -> Connection {
        Connection::open(self.lib.env.base.join("AtlasData/library/knowledge.sqlite")).unwrap()
    }

    fn count(&self, sql: &str) -> i64 {
        self.store().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn list(&self, query: SavedQuery) -> Vec<SavedItem> {
        saved::list(self.rec(), &query).unwrap().items
    }

    /// Asks `question`, answered with `parts`, and waits for the end.
    fn ask(&self, question: &str, parts: &[&str]) -> atlas_lib::domain::dto::AskStarted {
        self.chat
            .answer_with(&self.lib, streamed(&sse(parts), Duration::ZERO));
        let started = self.chat.started(&self.lib, question);
        let id = started.answer_id.clone();
        assert!(wait_until(LONG, || self.chat.ended(&id)));
        started
    }

    fn set_recording(&mut self, on: bool) {
        let mut settings = self.lib.root().settings().clone();
        settings.privacy.record_trails = on;
        self.lib.ws.update_settings(settings).unwrap();
    }
}

fn query() -> SavedQuery {
    SavedQuery {
        kind: None,
        filter: String::new(),
        document_id: None,
        page: 0,
        page_size: 25,
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
fn p3_15_bookmarks_are_built_in_rust_one_per_place() {
    let sv = Saved::new();
    let page = sv.search("Engineering marker 1");
    let (file, passage) = sv.file(&page, "book 000001.pdf");

    // From a result: at the hit's passage, the document as the catalog has it.
    let first = saved::save_bookmark(
        sv.rec(),
        &SaveTarget::File {
            file_id: file.clone(),
            place: FilePlace::Result,
        },
    )
    .unwrap();
    assert_eq!(first.anchor.ordinal, Some(sv.ordinal(passage.unwrap())));
    let (doc, hash): (String, Vec<u8>) = sv
        .lib
        .catalog()
        .query_row(
            "SELECT d.id, d.content_hash FROM documents d JOIN file_instances f ON f.document_id = d.id
             WHERE f.id = ?1",
            [&file],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (
            first.anchor.document_id.as_str(),
            first.anchor.content_hash.as_str()
        ),
        (doc.as_str(), to_hex(&hash).as_str())
    );
    assert_eq!(first.resolved.state, AnchorState::Found);

    // The same place again, and the same place from the trail's step: one
    // bookmark, refreshed.
    sv.later();
    let again = saved::save_bookmark(
        sv.rec(),
        &SaveTarget::File {
            file_id: file.clone(),
            place: FilePlace::Result,
        },
    )
    .unwrap();
    assert_eq!(again.id, first.id);
    assert!(again.updated_at > first.updated_at);
    let trail = sv.rec().status().current_trail_id.unwrap();
    let step = sv.rec().get(&trail).unwrap().steps[0].id.clone();
    let index = match &sv.rec().get(&trail).unwrap().steps[0].payload {
        StepPayload::Search(s) => s
            .results
            .iter()
            .position(|r| r.anchor.document_id == doc)
            .unwrap() as u32,
        other => panic!("{other:?}"),
    };
    let from_step = saved::save_bookmark(
        sv.rec(),
        &SaveTarget::Step {
            step_id: step.clone(),
            index,
        },
    )
    .unwrap();
    assert_eq!(from_step.id, first.id, "one bookmark per place");
    assert_eq!(sv.count("SELECT count(*) FROM bookmarks"), 1);

    // A page and the whole document of another book: two places, two
    // bookmarks.
    let other = sv.file_id("book 000002.pdf");
    let on_page = saved::save_bookmark(
        sv.rec(),
        &SaveTarget::File {
            file_id: other.clone(),
            place: FilePlace::Page { page_no: 1 },
        },
    )
    .unwrap();
    assert_eq!(on_page.anchor.page_no, Some(1));
    assert!(on_page.anchor.excerpt.is_some());
    let whole = saved::save_bookmark(
        sv.rec(),
        &SaveTarget::File {
            file_id: other,
            place: FilePlace::Document,
        },
    )
    .unwrap();
    assert_eq!(
        (whole.anchor.ordinal, whole.anchor.excerpt.as_ref()),
        (None, None)
    );
    assert_ne!(whole.id, on_page.id);
    assert_eq!(sv.count("SELECT count(*) FROM bookmarks"), 3);

    // From a citation: the passage as it was sent.
    let started = sv.ask("Biology marker 5", &["Cells divide [1]."]);
    let source = &started.sources[0];
    let cited = saved::save_bookmark(
        sv.rec(),
        &SaveTarget::File {
            file_id: source.file_instance_id.clone(),
            place: FilePlace::Citation {
                answer_id: started.answer_id.clone(),
                n: 1,
            },
        },
    )
    .unwrap();
    assert_eq!(
        cited.anchor.excerpt.as_deref(),
        Some(source.excerpt.as_str())
    );
    assert_eq!(
        cited.anchor.ordinal,
        Some(sv.ordinal(source.passage.unwrap()))
    );

    // Something no longer there is refused.
    assert_eq!(
        code(saved::save_bookmark(
            sv.rec(),
            &SaveTarget::Step {
                step_id: "gone".into(),
                index: 0
            }
        )),
        ErrorCode::SaveSourceGone
    );

    // After the file moves and is scanned again, the bookmark follows it.
    let books = sv.lib.env.base.join("Books");
    let path = find(&books, "book 000001.pdf").unwrap();
    fs::create_dir_all(books.join("Archive")).unwrap();
    fs::rename(&path, books.join("Archive/book 000001.pdf")).unwrap();
    let root_id: String = sv
        .lib
        .catalog()
        .query_row("SELECT id FROM source_roots", [], |r| r.get(0))
        .unwrap();
    inventory::control::enqueue(sv.lib.root(), &root_id).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(wait_until(LONG, || {
        let root = sv.lib.root();
        let inv = inventory::control::overview(root).unwrap();
        let ov = root
            .reader()
            .read(|c| extraction_db::overview(c, None))
            .unwrap()
            .unwrap();
        inv.active.is_none() && ov.active.is_none() && ov.queued_jobs == 0 && ov.counts.pending == 0
    }));
    let moved = sv
        .list(SavedQuery {
            document_id: Some(doc.clone()),
            ..query()
        })
        .into_iter()
        .find_map(|i| match i {
            SavedItem::Bookmark(b) => Some(b),
            SavedItem::Finding(_) => None,
        })
        .unwrap();
    assert_eq!(moved.resolved.state, AnchorState::Found);
    assert_ne!(
        moved.resolved.file_instance_id.as_deref(),
        Some(file.as_str())
    );
    assert_eq!(
        moved.resolved.file_instance_id,
        Some(sv.file_id("book 000001.pdf"))
    );
}

#[test]
fn p3_15_findings_keep_the_answer_or_passage_as_it_was() {
    let mut sv = Saved::new();

    // An answer, recorded: its question, the delivered text, the cited
    // sources only, and its step.
    let started = sv.ask("Biology marker 5", &["Cells **divide** [1]."]);
    assert!(started.sources.len() > 1, "more sources than were cited");
    assert!(wait_until(LONG, || sv.rec().status().step_count == 1));
    let trail = sv.rec().status().current_trail_id.unwrap();
    let step = sv.rec().get(&trail).unwrap().steps[0].id.clone();
    let finding = saved::save_finding(
        sv.rec(),
        &FindingSource::Answer {
            answer_id: started.answer_id.clone(),
        },
    )
    .unwrap();
    assert_eq!(finding.kind, FindingKind::Answer);
    assert_eq!(finding.question.as_deref(), Some("Biology marker 5"));
    assert_eq!(finding.text, "Cells **divide** [1].");
    assert_eq!(finding.sources.iter().map(|s| s.n).collect::<Vec<_>>(), [1]);
    assert_eq!(
        finding.sources[0].anchor.excerpt.as_deref(),
        Some(started.sources[0].excerpt.as_str())
    );
    assert_eq!(finding.from_step_id.as_deref(), Some(step.as_str()));

    // Paused, the answer on screen can still be kept; it has no step.
    sv.set_recording(false);
    let paused = sv.ask("Engineering marker 1", &["Gears mesh [1]."]);
    let kept = saved::save_finding(
        sv.rec(),
        &FindingSource::Answer {
            answer_id: paused.answer_id.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        (kept.text.as_str(), kept.from_step_id),
        ("Gears mesh [1].", None)
    );

    // A newer question: the earlier answer is gone from memory, but its
    // recorded step can still be kept.
    assert_eq!(
        code(saved::save_finding(
            sv.rec(),
            &FindingSource::Answer {
                answer_id: started.answer_id.clone()
            }
        )),
        ErrorCode::SaveSourceGone
    );
    let from_step = saved::save_finding(
        sv.rec(),
        &FindingSource::Step {
            step_id: step.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        (from_step.text.as_str(), from_step.sources.len()),
        ("Cells **divide** [1].", 1)
    );

    // A passage: its text and its one source.
    sv.set_recording(true);
    let page = sv.search("Mathematics marker 2");
    let (file, _) = sv.file(&page, "book 000002.pdf");
    let passage = saved::save_finding(
        sv.rec(),
        &FindingSource::Passage {
            target: SaveTarget::File {
                file_id: file,
                place: FilePlace::Result,
            },
        },
    )
    .unwrap();
    assert_eq!(passage.kind, FindingKind::Passage);
    // Over IPC an item says what it is, and a finding keeps its own kind.
    let json = serde_json::to_value(SavedItem::Finding(passage.clone())).unwrap();
    assert_eq!(
        (json["item"].as_str(), json["kind"].as_str()),
        (Some("finding"), Some("passage"))
    );
    let back: SavedItem = serde_json::from_value(json).unwrap();
    assert!(matches!(back, SavedItem::Finding(f) if f.id == passage.id));
    assert_eq!(
        Some(&passage.text),
        passage.sources[0].anchor.excerpt.as_ref()
    );
    assert!(passage.text.contains("Mathematics marker 2"));

    // A deleted step can't be kept any more; the finding saved from it
    // stays whole.
    sv.rec().delete_step(&step).unwrap();
    assert_eq!(
        code(saved::save_finding(
            sv.rec(),
            &FindingSource::Step { step_id: step }
        )),
        ErrorCode::SaveSourceGone
    );
    let all = sv.list(SavedQuery {
        kind: Some(SavedKind::Finding),
        ..query()
    });
    assert_eq!(all.len(), 4);
    assert!(all.iter().any(
        |i| matches!(i, SavedItem::Finding(f) if f.id == finding.id && f.text == finding.text)
    ));
}

#[test]
fn p3_15_notes_are_capped_and_the_list_filters() {
    let sv = Saved::new();
    let page = sv.search("Engineering marker 1");
    let (file, _) = sv.file(&page, "book 000001.pdf");
    let b = saved::save_bookmark(
        sv.rec(),
        &SaveTarget::File {
            file_id: file,
            place: FilePlace::Result,
        },
    )
    .unwrap();

    // Notes: at the limit, over it, cleared.
    let at_limit = "a".repeat(4000);
    let SavedItem::Bookmark(noted) =
        saved::set_note(sv.rec(), SavedKind::Bookmark, &b.id, &at_limit).unwrap()
    else {
        panic!()
    };
    assert_eq!(noted.note.as_deref(), Some(at_limit.as_str()));
    assert_eq!(
        code(saved::set_note(
            sv.rec(),
            SavedKind::Bookmark,
            &b.id,
            &"a".repeat(4001)
        )),
        ErrorCode::TextTooLong
    );
    let SavedItem::Bookmark(cleared) =
        saved::set_note(sv.rec(), SavedKind::Bookmark, &b.id, "   ").unwrap()
    else {
        panic!()
    };
    assert_eq!(cleared.note, None);
    assert_eq!(
        code(saved::set_note(sv.rec(), SavedKind::Finding, "gone", "x")),
        ErrorCode::SaveSourceGone
    );

    // A second item, later, with a note to find.
    sv.later();
    let page = sv.search("Mathematics marker 2");
    let (file, _) = sv.file(&page, "book 000002.pdf");
    let f = saved::save_finding(
        sv.rec(),
        &FindingSource::Passage {
            target: SaveTarget::File {
                file_id: file,
                place: FilePlace::Result,
            },
        },
    )
    .unwrap();
    sv.later();
    saved::set_note(
        sv.rec(),
        SavedKind::Finding,
        &f.id,
        "Café crème, then proofs",
    )
    .unwrap();

    let ids = |q: SavedQuery| -> Vec<String> {
        sv.list(q)
            .into_iter()
            .map(|i| match i {
                SavedItem::Bookmark(b) => b.id,
                SavedItem::Finding(f) => f.id,
            })
            .collect()
    };
    assert_eq!(ids(query()), [f.id.clone(), b.id.clone()], "newest first");
    assert_eq!(
        ids(SavedQuery {
            kind: Some(SavedKind::Bookmark),
            ..query()
        }),
        vec![b.id.clone()]
    );
    assert_eq!(
        ids(SavedQuery {
            filter: "CAFE".into(),
            ..query()
        }),
        vec![f.id.clone()],
        "folded"
    );
    assert_eq!(
        ids(SavedQuery {
            document_id: Some(b.anchor.document_id.clone()),
            ..query()
        }),
        vec![b.id.clone()]
    );
    let paged = saved::list(
        sv.rec(),
        &SavedQuery {
            page: 1,
            page_size: 1,
            ..query()
        },
    )
    .unwrap();
    assert_eq!((paged.total, paged.items.len()), (2, 1));

    // Deleting takes it out of its collections too.
    sv.store()
        .execute_batch(&format!(
            "INSERT INTO collections (id, kind, name, created_at, updated_at)
               VALUES ('c1', 'interest', 'Proofs', 't', 't');
             INSERT INTO collection_items (collection_id, item_kind, item_id, added_at)
               VALUES ('c1', 'finding', '{}', 't'), ('c1', 'bookmark', '{}', 't');",
            f.id, b.id
        ))
        .unwrap();
    saved::delete(sv.rec(), SavedKind::Finding, &f.id).unwrap();
    assert_eq!(sv.count("SELECT count(*) FROM findings"), 0);
    assert_eq!(sv.count("SELECT count(*) FROM collection_items"), 1);
    assert_eq!(
        code(saved::delete(sv.rec(), SavedKind::Finding, &f.id)),
        ErrorCode::SaveSourceGone
    );
    // Saving was never a trail step.
    let trails = sv
        .rec()
        .list(&TrailQuery {
            filter: String::new(),
            page: 0,
            page_size: 10,
        })
        .unwrap();
    assert_eq!(trails.trails[0].step_count, 2, "the two searches only");
}
