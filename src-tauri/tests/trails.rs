//! Slice 3.3: trails without a library to search (phase3-spec §7.1, §12).
//! Steps come from the recorder with empty result pages, and time from a
//! settable clock.
//!
//! - P3-11: sessions: a step after 29 and after 31 minutes, New trail,
//!   Continue, and a relaunch inside the window.
//! - P3-12: expiry at 90 days: unpinned, pinned, in a collection, and
//!   switched off; findings and bookmarks stay.
//! - P3-14: deleting a step, a trail and all trails; a finding saved from a
//!   deleted step stays whole.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use atlas_lib::domain::dto::{
    RecordingState, RecordingStatus, SearchIntent, SearchPage, SearchQuery, SemanticStatus,
    StepPayload, TrailQuery,
};
use atlas_lib::error::ErrorCode;
use atlas_lib::infrastructure::clock::{Clock, TestClock};
use atlas_lib::services::knowledge::Recorder;
use atlas_lib::services::workspace::Workspace;
use rusqlite::Connection;
use time::Duration;
use time::macros::datetime;

mod common;
use common::{Env, code};

type Statuses = Arc<Mutex<Vec<RecordingStatus>>>;

struct Lib {
    env: Env,
    clock: Arc<TestClock>,
    ws: Workspace,
    statuses: Statuses,
}

impl Lib {
    fn new() -> Self {
        let env = Env::new();
        let clock = Arc::new(TestClock::at(datetime!(2026-11-11 09:00 UTC)));
        let statuses = Statuses::default();
        let mut ws = Self::workspace(&env, &clock, &statuses);
        ws.choose(&env.base.join("AtlasData")).unwrap();
        Self {
            env,
            clock,
            ws,
            statuses,
        }
    }

    fn workspace(env: &Env, clock: &Arc<TestClock>, statuses: &Statuses) -> Workspace {
        let sink = Arc::clone(statuses);
        env.workspace()
            .with_clock(clock.clone() as Arc<dyn Clock>)
            .with_trail_events(Arc::new(move |s: &RecordingStatus| {
                sink.lock().unwrap().push(s.clone())
            }))
    }

    /// Closes the library and opens it again, as a relaunch does.
    fn relaunch(&mut self) {
        self.ws.shutdown();
        self.ws = Self::workspace(&self.env, &self.clock, &self.statuses);
        self.ws.startup();
        assert!(self.ws.open_root().is_some());
    }

    fn root(&self) -> PathBuf {
        self.env.base.join("AtlasData")
    }

    fn recorder(&self) -> &Arc<Recorder> {
        self.ws.require_open().unwrap().recorder()
    }

    fn later(&self, minutes: i64) {
        self.clock.advance(Duration::minutes(minutes));
    }

    /// A submitted search for `words`, finding nothing.
    fn search(&self, words: &str) {
        self.recorder()
            .search(&query(words), SearchIntent::Submit, &empty_page());
    }

    /// The trail the last step went into: the current one. (The list's
    /// order ties for trails made in the same second.)
    fn last_trail(&self) -> String {
        self.recorder().status().current_trail_id.unwrap()
    }

    /// Trail ids, newest activity first.
    fn trails(&self) -> Vec<String> {
        self.recorder()
            .list(&TrailQuery {
                filter: String::new(),
                page: 0,
                page_size: 100,
            })
            .unwrap()
            .trails
            .into_iter()
            .map(|t| t.id)
            .collect()
    }

    fn store(&self) -> Connection {
        Connection::open(self.root().join("library/knowledge.sqlite")).unwrap()
    }

    fn set_privacy(&mut self, change: impl FnOnce(&mut atlas_lib::domain::dto::PrivacySettings)) {
        let mut settings = self.ws.settings().unwrap();
        change(&mut settings.privacy);
        self.ws.update_settings(settings).unwrap();
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

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn words_of(lib: &Lib, trail: &str) -> Vec<String> {
    lib.recorder()
        .get(trail)
        .unwrap()
        .steps
        .into_iter()
        .map(|s| match s.payload {
            StepPayload::Search(s) => s.words,
            StepPayload::Note(n) => format!("note: {}", n.text),
            other => panic!("{other:?}"),
        })
        .collect()
}

#[test]
fn p3_11_sessions_last_thirty_minutes_and_survive_a_relaunch() {
    let mut lib = Lib::new();
    let status = lib.recorder().status();
    assert_eq!(status.state, RecordingState::Recording);
    assert_eq!(status.current_trail_id, None);
    assert!(!status.first_step_note, "nothing recorded yet");

    lib.search("stoic letters");
    let a = lib.last_trail();
    let status = lib.recorder().status();
    assert_eq!(status.current_trail_id.as_deref(), Some(a.as_str()));
    assert_eq!(status.step_count, 1);
    assert!(status.first_step_note, "Home's note, after the first step");

    // 29 minutes later: the same trail.
    lib.later(29);
    lib.search("seneca on anger");
    assert_eq!(lib.trails(), vec![a.clone()]);

    // 31 minutes after that: a new one.
    lib.later(31);
    lib.search("marcus aurelius");
    let b = lib.last_trail();
    assert_ne!(a, b);

    // New trail: the next step starts one, and no empty trail is made.
    let status = lib.recorder().start().unwrap();
    assert_eq!(status.current_trail_id, None);
    assert_eq!(lib.trails().len(), 2);
    lib.later(1);
    lib.search("epictetus");
    let c = lib.last_trail();
    assert!(c != a && c != b);

    // Continue an old trail: new steps join it.
    lib.later(1);
    lib.recorder().continue_trail(&a).unwrap();
    lib.search("the enchiridion");
    assert_eq!(lib.last_trail(), a);

    // A relaunch ten minutes later: still the same trail.
    lib.later(10);
    lib.relaunch();
    assert_eq!(
        lib.recorder().status().current_trail_id.as_deref(),
        Some(a.as_str())
    );
    lib.search("discourses");
    assert_eq!(lib.last_trail(), a);
    assert_eq!(
        words_of(&lib, &a),
        [
            "stoic letters",
            "seneca on anger",
            "the enchiridion",
            "discourses"
        ]
    );
    assert_eq!(words_of(&lib, &b), ["marcus aurelius"]);
    assert_eq!(words_of(&lib, &c), ["epictetus"]);

    // Continuing a trail that isn't there says so.
    assert_eq!(
        code(lib.recorder().continue_trail("gone")),
        ErrorCode::TrailNotFound
    );

    // Home's note goes once dismissed, and stays gone.
    assert!(!lib.recorder().dismiss_note().unwrap().first_step_note);
    lib.search("meditations");
    assert!(!lib.recorder().status().first_step_note);

    // The status went out after each step.
    let sent = lib.statuses.lock().unwrap();
    assert!(sent.iter().any(|s| s.step_count == 4));
}

#[test]
fn p3_11_the_same_search_again_updates_its_step() {
    let lib = Lib::new();
    lib.search("stoic letters");
    lib.later(2);
    lib.search("stoic letters");
    let trail = lib.last_trail();
    let detail = lib.recorder().get(&trail).unwrap();
    assert_eq!(detail.steps.len(), 1);
    assert_eq!(detail.steps[0].at, "2026-11-11T09:02:00Z");
    // Paging and refreshing are never steps.
    lib.recorder()
        .search(&query("other words"), SearchIntent::Refresh, &empty_page());
    lib.recorder()
        .search(&query("   "), SearchIntent::Submit, &empty_page());
    assert_eq!(lib.recorder().get(&trail).unwrap().steps.len(), 1);
}

#[test]
fn p3_12_only_unpinned_trails_outside_a_collection_expire_after_90_days() {
    let mut lib = Lib::new();
    let mut ids = Vec::new();
    for words in ["kept by a pin", "kept in a project", "left to expire"] {
        lib.recorder().start().unwrap();
        lib.search(words);
        ids.push(lib.last_trail());
    }
    let (pinned, collected, plain) = (&ids[0], &ids[1], &ids[2]);
    lib.recorder().pin(pinned, true).unwrap();
    let store = lib.store();
    store
        .execute_batch(&format!(
            "INSERT INTO collections (id, kind, name, created_at, updated_at)
               VALUES ('thesis', 'project', 'Thesis', 't', 't');
             INSERT INTO collection_items (collection_id, item_kind, item_id, added_at)
               VALUES ('thesis', 'trail', '{collected}', 't');
             INSERT INTO bookmarks (id, anchor, document_id, content_hash, created_at, updated_at)
               VALUES ('b1', '{{}}', 'doc', x'00', 't', 't');
             INSERT INTO findings (id, kind, text, sources, created_at, updated_at)
               VALUES ('f1', 'passage', 'kept', '[]', 't', 't');"
        ))
        .unwrap();

    // Inside 90 days nothing goes, even once a day has passed.
    lib.later(60 * 24 * 89);
    assert_eq!(
        lib.ws.require_open().unwrap().knowledge().expire_if_due(),
        Some(0)
    );
    assert_eq!(lib.trails().len(), 3);
    // It runs at most daily.
    lib.later(60 * 24 + 2);
    assert_eq!(
        lib.ws.require_open().unwrap().knowledge().expire_if_due(),
        Some(1)
    );
    assert_eq!(
        lib.ws.require_open().unwrap().knowledge().expire_if_due(),
        None
    );
    let left = lib.trails();
    assert!(left.contains(pinned) && left.contains(collected));
    assert!(!left.contains(plain));
    assert_eq!(count(&store, "SELECT count(*) FROM bookmarks"), 1);
    assert_eq!(count(&store, "SELECT count(*) FROM findings"), 1);
    assert_eq!(count(&store, "SELECT count(*) FROM trail_steps"), 2);

    // Switched off, nothing expires, at open or later.
    lib.recorder().pin(pinned, false).unwrap();
    lib.set_privacy(|p| p.expire_trails = false);
    lib.later(60 * 24 * 2);
    lib.relaunch();
    assert_eq!(
        lib.ws.require_open().unwrap().knowledge().expire_if_due(),
        None
    );
    assert_eq!(lib.trails().len(), 2);
    // Back on, the unpinned one goes at the next open.
    lib.set_privacy(|p| p.expire_trails = true);
    lib.relaunch();
    assert_eq!(lib.trails(), vec![collected.clone()]);
}

#[test]
fn p3_14_deleting_steps_and_trails_keeps_findings_whole() {
    let lib = Lib::new();
    lib.search("first words");
    lib.later(1);
    lib.search("second words");
    let trail = lib.last_trail();
    lib.recorder()
        .note(&trail, None, "  a note of my own  ")
        .unwrap();
    let steps = lib.recorder().get(&trail).unwrap().steps;
    assert_eq!(steps.len(), 3);
    assert_eq!(words_of(&lib, &trail)[2], "note: a note of my own");

    // A finding saved from the second step keeps its own copy.
    let store = lib.store();
    let sources = serde_json::to_string(&steps[1].payload).unwrap();
    store
        .execute(
            "INSERT INTO findings (id, kind, text, sources, created_at, updated_at)
             VALUES ('f1', 'answer', 'what I kept', ?1, 't', 't')",
            [&sources],
        )
        .unwrap();
    store
        .execute_batch(&format!(
            "INSERT INTO collections (id, kind, name, created_at, updated_at)
               VALUES ('stoics', 'interest', 'Stoics', 't', 't');
             INSERT INTO collection_items (collection_id, item_kind, item_id, added_at)
               VALUES ('stoics', 'trail', '{trail}', 't'),
                      ('stoics', 'finding', 'f1', 't');"
        ))
        .unwrap();
    let before: String = store
        .query_row("SELECT sources FROM findings WHERE id = 'f1'", [], |r| {
            r.get(0)
        })
        .unwrap();

    // A step: gone, with what it touched; the trail's counts follow.
    lib.recorder().delete_step(&steps[1].id).unwrap();
    let detail = lib.recorder().get(&trail).unwrap();
    assert_eq!(detail.trail.step_count, 2);
    assert_eq!(
        words_of(&lib, &trail),
        ["first words", "note: a note of my own"]
    );
    let text: String = store
        .query_row("SELECT search_text FROM trails", [], |r| r.get(0))
        .unwrap();
    assert!(text.contains("first words") && !text.contains("second"));
    assert_eq!(
        code(lib.recorder().delete_step(&steps[1].id)),
        ErrorCode::TrailNotFound
    );

    // A note too long is refused; one on a step is kept with it.
    assert_eq!(
        code(
            lib.recorder()
                .note(&trail, Some(&steps[0].id), &"a".repeat(4001))
        ),
        ErrorCode::TextTooLong
    );
    lib.recorder()
        .note(&trail, Some(&steps[0].id), "why I searched")
        .unwrap();
    assert_eq!(
        lib.recorder().get(&trail).unwrap().steps[0].note.as_deref(),
        Some("why I searched")
    );

    // Deleting a trail's last steps deletes the trail and its place in
    // the interest; the finding stays, whole and in the interest.
    lib.recorder().delete_step(&steps[0].id).unwrap();
    lib.recorder().delete_step(&steps[2].id).unwrap();
    assert!(lib.trails().is_empty());
    assert_eq!(count(&store, "SELECT count(*) FROM trail_steps"), 0);
    assert_eq!(count(&store, "SELECT count(*) FROM step_documents"), 0);
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM collection_items WHERE item_kind = 'trail'"
        ),
        0
    );
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM collection_items WHERE item_kind = 'finding'"
        ),
        1
    );
    let after: String = store
        .query_row("SELECT sources FROM findings WHERE id = 'f1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(after, before);

    // A trail, then all of them.
    for words in ["one", "two", "three"] {
        lib.recorder().start().unwrap();
        lib.search(words);
    }
    let ids = lib.trails();
    lib.recorder().delete(&ids[0]).unwrap();
    assert_eq!(
        code(lib.recorder().delete(&ids[0])),
        ErrorCode::TrailNotFound
    );
    assert_eq!(lib.trails().len(), 2);
    assert_eq!(lib.recorder().delete_all().unwrap(), 2);
    assert!(lib.trails().is_empty());
    assert_eq!(lib.recorder().status().current_trail_id, None);
    assert_eq!(count(&store, "SELECT count(*) FROM findings"), 1);
}

#[test]
fn p3_14_titles_pins_and_the_filter() {
    let lib = Lib::new();
    lib.search("Café society");
    let trail = lib.last_trail();
    let renamed = lib
        .recorder()
        .rename(&trail, "  Paris in the 1920s  ")
        .unwrap();
    assert_eq!(renamed.title.as_deref(), Some("Paris in the 1920s"));
    assert_eq!(renamed.label, "Paris in the 1920s");
    assert!(lib.recorder().pin(&trail, true).unwrap().pinned);
    let find = |filter: &str| {
        lib.recorder()
            .list(&TrailQuery {
                filter: filter.into(),
                page: 0,
                page_size: 10,
            })
            .unwrap()
            .total
    };
    // Folded: accents and case don't matter; titles and searches count.
    assert_eq!(find("cafe"), 1);
    assert_eq!(find("PARIS"), 1);
    assert_eq!(find("berlin"), 0);
    assert_eq!(find("100%"), 0);
    // An empty title shows the first search again.
    let untitled = lib.recorder().rename(&trail, "").unwrap();
    assert_eq!(
        (untitled.title, untitled.label.as_str()),
        (None, "Café society")
    );
    assert_eq!(
        code(lib.recorder().rename(&trail, &"t".repeat(201))),
        ErrorCode::TextTooLong
    );
}
