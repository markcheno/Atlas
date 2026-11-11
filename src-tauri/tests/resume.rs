//! Slice 3.4, P3-13: the resume scenario (phase3-spec §10), as the script
//! `fixtures/knowledge/resume-scenario.json` lays it out, on every OS.
//!
//! A session is recorded. Twelve weeks pass: its pinned trail is kept, an
//! older unpinned one is not. Meanwhile a cited file moves, another is
//! renamed, a third is deleted, a book is read again by a newer reader that
//! splits its text differently, and the embedding model changes. After a
//! restart every step shows in order, the answer reads as it did, each
//! source resolves as the script says, what was kept is intact, and Search
//! again and Ask again add steps to the continued trail.
//!
//! The bookmark and the finding are saved through their service (Slice
//! 3.5), and the interest through its own (Slice 3.6).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_lib::domain::dto::{
    AnchorState, CollectionInput, CollectionItemRef, CollectionKind, FindingSource, OpenAction,
    OpenPlace, OpenVia, RecordingStatus, SaveTarget, SavedItem, SavedKind, SavedQuery,
    SearchIntent, StepPayload, TrailDetail, TrailQuery,
};
use atlas_lib::infrastructure::clock::{Clock, TestClock};
use atlas_lib::services::embedding::{models, triggers};
use atlas_lib::services::extraction::db as extraction_db;
use atlas_lib::services::inventory;
use atlas_lib::services::knowledge::{Recorder, collections, saved};
use rusqlite::Connection;
use serde::Deserialize;
use time::macros::datetime;

mod common;
use common::chat::{Chat, slow, sse, streamed};
use common::meaning::{LONG, Lib};
use common::wait_until;

// ------------------------------------------------------------- the script

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Script {
    library: Library,
    earlier: Earlier,
    session: Vec<Step>,
    keep: Keep,
    later: Later,
    expect: Expect,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Library {
    corpus: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Earlier {
    days_before: i64,
    searches: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    search: Option<String>,
    open: Option<String>,
    ask: Option<String>,
    #[serde(default)]
    answer: Vec<String>,
    open_cited: Option<u32>,
    cited_file: Option<String>,
    #[serde(default)]
    stop_after_text: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Keep {
    pin: bool,
    bookmark: String,
    finding_from: String,
    interest: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Move {
    file: String,
    to: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Later {
    weeks: i64,
    #[serde(rename = "move")]
    moved: Move,
    rename: Move,
    delete: String,
    reread: String,
    embedding_dims: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Expect {
    kinds: Vec<String>,
    opened: BTreeMap<String, AnchorState>,
    earlier_trail: String,
    search_again: String,
    ask_again: String,
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn script() -> Script {
    let text = fs::read_to_string(repo().join("fixtures/knowledge/resume-scenario.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

// ------------------------------------------------------------ the harness

/// The file named `name` under `dir`.
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

struct Scenario {
    lib: Lib,
    clock: Arc<TestClock>,
    chat: Chat,
}

impl Scenario {
    fn recorder(&self) -> &Arc<Recorder> {
        self.lib.root().recorder()
    }

    fn steps(&self) -> u32 {
        self.recorder().status().step_count
    }

    fn later(&self, minutes: i64) {
        self.clock.advance(time::Duration::minutes(minutes));
    }

    fn catalog(&self) -> Connection {
        self.lib.catalog()
    }

    fn books(&self) -> PathBuf {
        self.lib.env.base.join("Books")
    }

    fn document_of(&self, display_name: &str) -> String {
        self.catalog()
            .query_row(
                "SELECT document_id FROM file_instances WHERE display_name = ?1",
                [display_name],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// A submitted search, as `search_documents` runs it; then the named
    /// file opened from its results.
    fn search(&self, words: &str, open: Option<&str>) {
        let q = Lib::query(words);
        let page = self.lib.hybrid(&q);
        self.recorder().search(&q, SearchIntent::Submit, &page);
        if let Some(name) = open {
            let hit = page
                .hits
                .iter()
                .find(|h| h.display_name == name)
                .unwrap_or_else(|| panic!("“{words}” didn't find {name}"));
            self.recorder().open(
                &hit.file_instance_id,
                &OpenVia::Result,
                OpenAction::Details,
                OpenPlace::default(),
            );
        }
    }

    fn trail(&self, id: &str) -> TrailDetail {
        self.recorder().get(id).unwrap()
    }

    fn settled(&self) -> bool {
        let root = self.lib.root();
        let inv = inventory::control::overview(root).unwrap();
        let ov = root
            .reader()
            .read(|c| extraction_db::overview(c, None))
            .unwrap()
            .unwrap();
        inv.active.is_none() && ov.active.is_none() && ov.queued_jobs == 0 && ov.counts.pending == 0
    }

    fn serving(&self) -> String {
        models::serving(&self.catalog()).unwrap().unwrap().id
    }
}

/// The book as an older reader left it: a Project Gutenberg header passage
/// first and every other passage one place later, at reader version 2. The
/// next open reads it again with version 3, which leaves the header out.
fn age(lib: &Lib, document: &str) {
    let doc = document.to_owned();
    lib.root()
        .catalog()
        .call(move |c| {
            c.execute_batch("BEGIN").unwrap();
            c.execute(
                "UPDATE passages SET ordinal = ordinal + 1000000 WHERE document_id = ?1",
                [&doc],
            )
            .unwrap();
            c.execute(
                "UPDATE passages SET ordinal = ordinal - 1000000 + 1 WHERE document_id = ?1",
                [&doc],
            )
            .unwrap();
            c.execute(
                "INSERT INTO passages (document_id, ordinal, page_no, location, text)
                 VALUES (?1, 0, NULL, 'Chapter 0, part 1',
                         '*** START OF THE PROJECT GUTENBERG EBOOK *** Produced by volunteers of Project Gutenberg.')",
                [&doc],
            )
            .unwrap();
            c.execute(
                "UPDATE documents SET extractor_version = 2, boilerplate_chars = 0 WHERE id = ?1",
                [&doc],
            )
            .unwrap();
            c.execute_batch("COMMIT").unwrap();
        })
        .unwrap();
}

fn payloads(t: &TrailDetail) -> Vec<String> {
    t.steps
        .iter()
        .map(|s| serde_json::to_string(&s.payload).unwrap())
        .collect()
}

fn kind(p: &StepPayload) -> &'static str {
    match p {
        StepPayload::Search(_) => "search",
        StepPayload::Ask(_) => "ask",
        StepPayload::Open(_) => "open",
        StepPayload::Note(_) => "note",
    }
}

#[test]
fn p3_13_a_session_resumed_after_twelve_weeks_of_change() {
    let script = script();
    let corpus = repo().join("spikes/extraction-bench/corpus");
    let extra: Vec<PathBuf> = script
        .library
        .corpus
        .iter()
        .map(|f| corpus.join(f))
        .collect();
    let extra_refs: Vec<&Path> = extra.iter().map(PathBuf::as_path).collect();

    // The library, read and embedded, at the time of the earlier session.
    let start = datetime!(2026-08-01 09:00 UTC);
    let clock = Arc::new(TestClock::at(start));
    let statuses: Arc<Mutex<Vec<RecordingStatus>>> = Arc::default();
    let (c, sink) = (clock.clone(), Arc::clone(&statuses));
    let lib = Lib::custom(&[], false, 0, &extra_refs, move |ws| {
        ws.with_clock(c as Arc<dyn Clock>)
            .with_trail_events(Arc::new(move |s: &RecordingStatus| {
                sink.lock().unwrap().push(s.clone())
            }))
    });
    let chat = Chat::new(&lib);
    let mut sc = Scenario { lib, clock, chat };
    let reread = sc.document_of(&script.later.reread);
    age(&sc.lib, &reread);

    // The earlier session: a trail that will expire.
    for words in &script.earlier.searches {
        sc.search(words, None);
    }
    let earlier = sc.recorder().status().current_trail_id.unwrap();
    sc.clock
        .advance(time::Duration::days(script.earlier.days_before));

    // The session.
    let mut opened: Vec<String> = Vec::new();
    let mut answers: BTreeMap<String, String> = BTreeMap::new();
    for step in &script.session {
        sc.later(2);
        let before = sc.steps();
        if let Some(words) = &step.search {
            sc.search(words, step.open.as_deref());
            opened.extend(step.open.clone());
            continue;
        }
        let question = step.ask.as_deref().unwrap();
        if step.stop_after_text {
            sc.chat.answer_with(&sc.lib, slow(40));
            let started = sc.chat.started(&sc.lib, question);
            let id = started.answer_id.clone();
            assert!(wait_until(LONG, || sc.chat.steps(&id).len() > 1));
            sc.lib.root().answers().cancel(&id);
        } else {
            let parts: Vec<&str> = step.answer.iter().map(String::as_str).collect();
            sc.chat
                .answer_with(&sc.lib, streamed(&sse(&parts), Duration::ZERO));
            let started = sc.chat.started(&sc.lib, question);
            assert!(wait_until(LONG, || sc.steps() == before + 1), "{question}");
            answers.insert(question.to_owned(), step.answer.concat());
            if let Some(n) = step.open_cited {
                let source = started.sources.iter().find(|s| s.n == n).unwrap();
                assert_eq!(
                    Some(&source.display_name),
                    step.cited_file.as_ref(),
                    "the cited source is the one the script names"
                );
                sc.recorder().open(
                    &source.file_instance_id,
                    &OpenVia::Citation {
                        answer_id: started.answer_id.clone(),
                        n,
                    },
                    OpenAction::Details,
                    OpenPlace::default(),
                );
                opened.extend(step.cited_file.clone());
            }
            continue;
        }
        assert!(wait_until(LONG, || sc.steps() == before + 1), "{question}");
    }
    let main = sc.recorder().status().current_trail_id.unwrap();
    assert_ne!(main, earlier);
    let recorded = sc.trail(&main);
    let kinds: Vec<&str> = recorded.steps.iter().map(|s| kind(&s.payload)).collect();
    assert_eq!(kinds, script.expect.kinds);

    // What the user kept.
    sc.recorder().pin(&main, script.keep.pin).unwrap();
    // The bookmark, the finding and the interest through their services.
    let bookmark_doc = sc.document_of(&script.keep.bookmark);
    let open_step = recorded
        .steps
        .iter()
        .find(
            |s| matches!(&s.payload, StepPayload::Open(o) if o.anchor.document_id == bookmark_doc),
        )
        .unwrap()
        .id
        .clone();
    let finding_step = recorded
        .steps
        .iter()
        .find(
            |s| matches!(&s.payload, StepPayload::Ask(a) if a.question == script.keep.finding_from),
        )
        .unwrap()
        .id
        .clone();
    let bookmark = saved::save_bookmark(
        sc.recorder(),
        &SaveTarget::Step {
            step_id: open_step,
            index: 0,
        },
    )
    .unwrap();
    saved::set_note(
        sc.recorder(),
        SavedKind::Bookmark,
        &bookmark.id,
        "Start here",
    )
    .unwrap();
    let finding = saved::save_finding(
        sc.recorder(),
        &FindingSource::Step {
            step_id: finding_step,
        },
    )
    .unwrap();
    let store_path = sc.lib.env.base.join("AtlasData/library/knowledge.sqlite");
    let stored = |bookmark: &str, finding: &str| -> (String, String) {
        Connection::open(&store_path)
            .unwrap()
            .query_row(
                "SELECT (SELECT anchor FROM bookmarks WHERE id = ?1),
                        (SELECT sources FROM findings WHERE id = ?2)",
                [bookmark, finding],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };
    let kept = stored(&bookmark.id, &finding.id);
    let interest = collections::save(
        sc.recorder(),
        &CollectionInput {
            id: None,
            kind: CollectionKind::Interest,
            name: script.keep.interest.clone(),
            description: None,
            goal: None,
            status: None,
        },
    )
    .unwrap();
    for item in [
        CollectionItemRef::Trail { id: main.clone() },
        CollectionItemRef::Bookmark {
            id: bookmark.id.clone(),
        },
        CollectionItemRef::Finding {
            id: finding.id.clone(),
        },
    ] {
        collections::add(sc.recorder(), &interest.id, &item).unwrap();
    }
    let before = payloads(&sc.trail(&main));

    // Twelve weeks pass, and the library changes.
    sc.clock.advance(time::Duration::weeks(script.later.weeks));
    let books = sc.books();
    let moved = find(&books, &script.later.moved.file).unwrap();
    fs::create_dir_all(books.join(&script.later.moved.to)).unwrap();
    fs::rename(
        &moved,
        books
            .join(&script.later.moved.to)
            .join(&script.later.moved.file),
    )
    .unwrap();
    let renamed = find(&books, &script.later.rename.file).unwrap();
    fs::rename(&renamed, renamed.with_file_name(&script.later.rename.to)).unwrap();
    fs::remove_file(find(&books, &script.later.delete).unwrap()).unwrap();
    let old_model = sc.serving();
    let dims = script.later.embedding_dims;
    sc.lib
        .root()
        .catalog()
        .call(move |c| {
            c.execute(
                "UPDATE model_roles SET dimensions = ?1 WHERE role = 'embedding'",
                [dims],
            )
        })
        .unwrap()
        .unwrap();
    triggers::changed(sc.lib.root()).unwrap();

    // The app restarts, and the folder is scanned again.
    sc.lib.ws.shutdown();
    sc.lib.ws.startup();
    let root_id: String = sc
        .catalog()
        .query_row("SELECT id FROM source_roots", [], |r| r.get(0))
        .unwrap();
    inventory::control::enqueue(sc.lib.root(), &root_id).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    if !wait_until(LONG, || sc.settled()) {
        let root = sc.lib.root();
        let inv = inventory::control::overview(root).unwrap();
        let ov = root
            .reader()
            .read(|c| extraction_db::overview(c, None))
            .unwrap()
            .unwrap();
        let pending: Vec<(String, String, String)> = sc
            .catalog()
            .prepare(
                "SELECT display_path, availability, extract_state FROM file_instances
                 WHERE extract_state = 'pending'",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        panic!("never settled: {pending:?}\n{inv:#?}\n{ov:#?}");
    }
    // The new model serves once it holds every document. (Its index loads
    // into memory at the first search.)
    let indexed = || {
        let ov = atlas_lib::services::embedding::status::overview(&sc.catalog(), false).unwrap();
        sc.serving() != old_model && ov.active.is_none() && ov.counts.waiting == 0
    };
    if !wait_until(LONG, indexed) {
        let c = sc.catalog();
        let models: Vec<(String, String, i64)> = c
            .prepare("SELECT id, status, dims FROM embedding_models")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let jobs: Vec<(String, String)> = c
            .prepare("SELECT kind, state FROM jobs ORDER BY created_at")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let ov = atlas_lib::services::embedding::status::overview(&c, false);
        panic!(
            "the new model never served: old {old_model}, serving {}, models {models:?}, jobs {jobs:?}\n{ov:#?}",
            sc.serving()
        );
    }

    // The earlier, unpinned trail expired; the pinned one is whole.
    let all = sc
        .recorder()
        .list(&TrailQuery {
            filter: String::new(),
            page: 0,
            page_size: 10,
        })
        .unwrap();
    assert_eq!(script.expect.earlier_trail, "expired");
    assert_eq!(
        all.trails.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        [main.as_str()]
    );
    let resumed = sc.trail(&main);
    assert_eq!(payloads(&resumed), before, "every step as it was, in order");
    for (question, text) in &answers {
        let shown = resumed.steps.iter().find_map(|s| match &s.payload {
            StepPayload::Ask(a) if &a.question == question => Some(a.answer.clone()),
            _ => None,
        });
        assert_eq!(shown.as_ref(), Some(text), "the answer reads as it did");
    }

    // Each source opened resolves as the script says.
    let opens: Vec<_> = resumed
        .steps
        .iter()
        .filter(|s| matches!(s.payload, StepPayload::Open(_)))
        .collect();
    assert_eq!(opens.len(), opened.len());
    for (step, name) in opens.iter().zip(&opened) {
        let want = script.expect.opened[name];
        let got = &step.anchors[0];
        assert_eq!(got.state, want, "{name}");
        if want == AnchorState::Found || want == AnchorState::FoundElsewhere {
            assert!(got.excerpt_found, "{name}: its passage was found");
        }
    }
    let at = |name: &str| {
        let i = opened.iter().position(|o| o == name).unwrap();
        opens[i].anchors[0].file_instance_id.clone().unwrap()
    };
    let (path, display): (String, String) = sc
        .catalog()
        .query_row(
            "SELECT display_path, display_name FROM file_instances WHERE id = ?1",
            [at(&script.later.moved.file)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(
        path.contains(&script.later.moved.to),
        "at the new place: {path}"
    );
    assert_eq!(display, script.later.moved.file);
    let display: String = sc
        .catalog()
        .query_row(
            "SELECT display_name FROM file_instances WHERE id = ?1",
            [at(&script.later.rename.file)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(display, script.later.rename.to);

    // What was kept is intact.
    assert_eq!(stored(&bookmark.id, &finding.id), kept);
    let held = collections::get(sc.recorder(), &interest.id).unwrap();
    assert_eq!(
        (held.trails.len(), held.bookmarks.len(), held.findings.len()),
        (1, 1, 1),
        "the interest holds them all"
    );
    let SavedItem::Bookmark(still) = saved::list(
        sc.recorder(),
        &SavedQuery {
            kind: Some(SavedKind::Bookmark),
            filter: String::new(),
            document_id: None,
            page: 0,
            page_size: 10,
        },
    )
    .unwrap()
    .items
    .remove(0) else {
        panic!("the bookmark")
    };
    assert_eq!(still.note.as_deref(), Some("Start here"));
    assert_eq!(
        still.resolved.state,
        AnchorState::Found,
        "at the moved file"
    );

    // Search again and Ask again: new steps on the continued trail.
    sc.recorder().continue_trail(&main).unwrap();
    sc.search(&script.expect.search_again, None);
    sc.chat.answer_with(
        &sc.lib,
        streamed(&sse(&["Cells divide [1]."]), Duration::ZERO),
    );
    let ask = resumed
        .steps
        .iter()
        .find_map(|s| match &s.payload {
            StepPayload::Ask(a) if a.question == script.expect.ask_again => Some(a.clone()),
            _ => None,
        })
        .unwrap();
    let count = before.len() as u32;
    sc.chat.ask_in(
        &sc.lib,
        &ask.question,
        ask.filters.source_root_id.clone(),
        ask.filters.file_type,
    );
    assert!(
        wait_until(LONG, || sc.steps() == count + 2),
        "both joined the trail"
    );
    let after = sc.trail(&main);
    assert_eq!(
        payloads(&after)[..before.len()],
        before[..],
        "the old steps stay as they were"
    );
    let new: Vec<&str> = after.steps[before.len()..]
        .iter()
        .map(|s| kind(&s.payload))
        .collect();
    assert_eq!(new, ["search", "ask"]);
}
