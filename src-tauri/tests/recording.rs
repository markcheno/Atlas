//! Slice 3.3: recording against a read, embedded library and a scripted
//! chat service (phase3-spec §7.1, §12).
//!
//! - P3-09: a submit, paging, a live refresh, a filter change and the same
//!   search again; questions done, stopped, failed and with nothing
//!   found; opens from a result, a citation and Related, and from the
//!   Library: exactly the steps §7.1 names, in order.
//! - P3-10: paused, nothing is written; with the store refusing writes,
//!   the search and the answer still succeed and the failure is reported
//!   once.
//!
//! The commands are thin: `search_documents` runs the search, then hands
//! the page to the recorder, as these tests do.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_lib::domain::dto::{
    AnswerErrorCode, AnswerEvent, AnswerStep, AskEnding, AskQuery, AskReply, AskStarted, FileType,
    OpenAction, OpenPlace, OpenVia, ProviderKind, ProviderLocation, ProviderProfileInput, Reached,
    RecordingStatus, SearchIntent, SearchPage, SearchQuery, StepPayload, TrailDetail, TrailQuery,
};
use atlas_lib::infrastructure::clock::{Clock, TestClock};
use atlas_lib::services::answers::stream::ChatLimits;
use atlas_lib::services::answers::{self, AnswerEvents, AskDeps};
use atlas_lib::services::knowledge::Recorder;
use atlas_lib::services::providers::{privacy, profiles, url_policy};
use atlas_lib::services::search::MeaningContext;
use rusqlite::Connection;
use serde_json::json;
use time::macros::datetime;

mod common;
use common::http::{MockServer, Reply, stream};
use common::meaning::{BREAD, LONG, Lib, SKY};
use common::wait_until;

const SSE: &str = "text/event-stream";
const NEAR_SKY: &str =
    "Distant comets and glowing nebulae drifted past the starship on its long voyage.";
const QUESTION: &str = "comets and glowing nebulae";

fn delta(text: &str) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices":[{"delta":{"content": text}}]})
    )
}

fn sse(parts: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = parts.iter().map(|p| delta(p)).collect();
    out.push("data: [DONE]\n\n".into());
    out
}

fn streamed(pieces: &[String], pause: Duration) -> Reply {
    let refs: Vec<&str> = pieces.iter().map(String::as_str).collect();
    stream(SSE, &refs, pause)
}

/// A library with a chat role whose service can be swapped per question.
struct Rec {
    lib: Lib,
    clock: Arc<TestClock>,
    profile: String,
    servers: Mutex<Vec<MockServer>>,
    events: Arc<Mutex<Vec<AnswerEvent>>>,
    statuses: Arc<Mutex<Vec<RecordingStatus>>>,
}

impl Rec {
    fn new() -> Self {
        let clock = Arc::new(TestClock::at(datetime!(2026-11-11 09:00 UTC)));
        let statuses: Arc<Mutex<Vec<RecordingStatus>>> = Arc::default();
        let (c, sink) = (clock.clone(), Arc::clone(&statuses));
        let lib = Lib::with_workspace(&[SKY, NEAR_SKY, BREAD], false, move |ws| {
            ws.with_clock(c as Arc<dyn Clock>)
                .with_trail_events(Arc::new(move |s: &RecordingStatus| {
                    sink.lock().unwrap().push(s.clone())
                }))
        });
        let saved = profiles::save(
            lib.root(),
            &lib.vault,
            ProviderProfileInput {
                id: None,
                kind: ProviderKind::OpenaiCompatible,
                label: "Chat service".into(),
                base_url: "http://127.0.0.1:9/v1".into(),
                api_key: None,
                clear_api_key: false,
                insecure_http_acknowledged: false,
            },
        )
        .unwrap();
        let profile = saved.id.clone();
        lib.root()
            .catalog()
            .call(move |c| {
                c.execute(
                    "INSERT OR REPLACE INTO model_roles (role, provider_profile_id, model_id,
                         dimensions, tested_at, updated_at)
                     VALUES ('chat', ?1, 'test-chat', NULL, 't', 't')",
                    [&profile],
                )
                .unwrap();
            })
            .unwrap();
        Self {
            lib,
            clock,
            profile: saved.id,
            servers: Mutex::default(),
            events: Arc::default(),
            statuses,
        }
    }

    fn recorder(&self) -> &Arc<Recorder> {
        self.lib.root().recorder()
    }

    /// The next questions are answered with `reply`.
    fn answer_with(&self, reply: Reply) {
        let server = MockServer::start(vec![("POST /v1/chat/completions", reply)]);
        let (id, url) = (self.profile.clone(), server.url("/v1"));
        self.lib
            .root()
            .catalog()
            .call(move |c| {
                c.execute(
                    "UPDATE provider_profiles SET base_url = ?2 WHERE id = ?1",
                    [&id, &url],
                )
                .unwrap();
            })
            .unwrap();
        self.servers.lock().unwrap().push(server);
    }

    /// What `search_documents` does.
    fn search(&self, q: &SearchQuery, intent: SearchIntent) -> SearchPage {
        let page = self.lib.hybrid(q);
        self.recorder().search(q, intent, &page);
        page
    }

    fn ask(&self, question: &str) -> AskReply {
        let root = self.lib.root();
        let log = Arc::clone(&self.events);
        let events: AnswerEvents =
            Arc::new(move |e: &AnswerEvent| log.lock().unwrap().push(e.clone()));
        let settings = root.settings().privacy.clone();
        answers::ask(
            &AskDeps {
                reader: root.reader(),
                meaning: MeaningContext {
                    vectors: root.vectors(),
                    catalog: &root.catalog().handle(),
                    embedder: &self.lib.embedder,
                    vault: &self.lib.vault,
                    allow_remote: privacy::remote_embedding(&settings),
                },
                vault: &self.lib.vault,
                privacy: &settings,
                locate: url_policy::location_of,
                limits: ChatLimits::default(),
                events,
                answers: root.answers(),
                recorder: Some(root.recorder()),
            },
            &AskQuery {
                question: question.into(),
                source_root_id: None,
                file_type: None,
            },
        )
        .unwrap()
    }

    fn started(&self, question: &str) -> AskStarted {
        match self.ask(question) {
            AskReply::Started(s) => s,
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    fn steps(&self, id: &str) -> Vec<AnswerStep> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.answer_id == id)
            .map(|e| e.step.clone())
            .collect()
    }

    fn ended(&self, id: &str) -> bool {
        self.steps(id).iter().any(|s| {
            matches!(
                s,
                AnswerStep::Done { .. } | AnswerStep::Stopped | AnswerStep::Error { .. }
            )
        })
    }

    /// Every step recorded, over every trail.
    fn recorded(&self) -> u32 {
        self.recorder().status().step_count
    }

    fn trail(&self) -> TrailDetail {
        let page = self
            .recorder()
            .list(&TrailQuery {
                filter: String::new(),
                page: 0,
                page_size: 10,
            })
            .unwrap();
        assert_eq!(page.total, 1, "one session, one trail");
        self.recorder().get(&page.trails[0].id).unwrap()
    }

    fn store(&self) -> Connection {
        Connection::open(self.lib.env.base.join("AtlasData/library/knowledge.sqlite")).unwrap()
    }

    fn changes(&self) -> i64 {
        self.store()
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'changes'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn later(&self) {
        self.clock.advance(time::Duration::minutes(1));
    }

    fn set_privacy(&mut self, change: impl FnOnce(&mut atlas_lib::domain::dto::PrivacySettings)) {
        let mut settings = self.lib.root().settings().clone();
        change(&mut settings.privacy);
        self.lib.ws.update_settings(settings).unwrap();
    }
}

fn kinds(trail: &TrailDetail) -> Vec<&'static str> {
    trail
        .steps
        .iter()
        .map(|s| match &s.payload {
            StepPayload::Search(_) => "search",
            StepPayload::Ask(_) => "ask",
            StepPayload::Open(_) => "open",
            StepPayload::Note(_) => "note",
        })
        .collect()
}

/// The ordinal of a catalog passage.
fn ordinal(lib: &Lib, passage: i64) -> u32 {
    lib.catalog()
        .query_row(
            "SELECT ordinal FROM passages WHERE id = ?1",
            [passage],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn p3_09_exactly_the_steps_section_7_1_names() {
    let rec = Rec::new();
    let catalog_changes = |rec: &Rec| -> i64 {
        rec.lib
            .catalog()
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get(0))
            .unwrap()
    };
    let jobs_before = catalog_changes(&rec);

    // Searches: a submit; then paging, a refresh and a filter change are
    // not steps; the same submit again updates the first.
    let q = Lib::query("comets nebulae");
    let first = rec.search(&q, SearchIntent::Submit);
    assert!(!first.hits.is_empty());
    rec.search(
        &SearchQuery {
            page: 1,
            ..q.clone()
        },
        SearchIntent::Refresh,
    );
    rec.search(&q, SearchIntent::Refresh);
    rec.search(
        &SearchQuery {
            file_type: Some(FileType::Pdf),
            ..q.clone()
        },
        SearchIntent::Refresh,
    );
    assert_eq!(rec.recorded(), 1);
    rec.later();
    rec.search(&q, SearchIntent::Submit);
    assert_eq!(rec.recorded(), 1, "the same search updates its step");
    // In one folder: another step, with the folder's name as it was.
    let folder: String = rec
        .lib
        .catalog()
        .query_row("SELECT id FROM source_roots", [], |r| r.get(0))
        .unwrap();
    rec.search(
        &SearchQuery {
            source_root_id: Some(folder.clone()),
            ..q.clone()
        },
        SearchIntent::Submit,
    );
    assert_eq!(rec.recorded(), 2);

    // A question that is answered, then opening its citation.
    rec.answer_with(streamed(
        &sse(&["The starship passed comets [1]", " and nebulae."]),
        Duration::ZERO,
    ));
    let done = rec.started(QUESTION);
    assert!(
        wait_until(LONG, || rec.recorded() == 3),
        "the answer was recorded"
    );
    assert!(rec.ended(&done.answer_id));
    let cited = &done.sources[0];
    rec.recorder().open(
        &cited.file_instance_id,
        &OpenVia::Citation {
            answer_id: done.answer_id.clone(),
            n: 1,
        },
        OpenAction::Details,
        OpenPlace::default(),
    );
    assert_eq!(rec.recorded(), 4);

    // Stopped once text has come.
    let parts: Vec<String> = (0..40).map(|i| format!("word{i} ")).collect();
    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    rec.answer_with(streamed(&sse(&refs), Duration::from_millis(100)));
    let stopped = rec.started(QUESTION);
    assert!(wait_until(LONG, || rec.steps(&stopped.answer_id).len() > 1));
    rec.lib.root().answers().cancel(&stopped.answer_id);
    assert!(wait_until(LONG, || rec.recorded() == 5));

    // Failed midway.
    rec.answer_with(streamed(
        &[
            delta("The starship passed comets [1]"),
            format!(
                "data: {}\n\n",
                json!({"error": {"message": "out of memory"}})
            ),
        ],
        Duration::ZERO,
    ));
    let failed = rec.started(QUESTION);
    assert!(wait_until(LONG, || rec.recorded() == 6));
    assert!(rec.ended(&failed.answer_id));

    // Nothing found: the model isn't asked, and it is still a step.
    assert!(matches!(
        rec.ask("zqxjv plorbix"),
        AskReply::NothingFound { .. }
    ));
    assert_eq!(rec.recorded(), 7);

    // Opens: from a result, twice (the details, then Open: one step);
    // from Related; from the Library, never.
    let page = rec.search(&q, SearchIntent::Submit);
    let hit = &page.hits[0];
    for action in [OpenAction::Details, OpenAction::Open, OpenAction::Details] {
        rec.recorder().open(
            &hit.file_instance_id,
            &OpenVia::Result,
            action,
            OpenPlace::default(),
        );
    }
    let other = page
        .hits
        .iter()
        .find(|h| h.document_id != hit.document_id)
        .unwrap();
    rec.recorder().open(
        &other.file_instance_id,
        &OpenVia::Related,
        OpenAction::Details,
        OpenPlace::default(),
    );
    rec.recorder().open(
        &other.file_instance_id,
        &OpenVia::Library,
        OpenAction::Open,
        OpenPlace::default(),
    );

    let trail = rec.trail();
    assert_eq!(
        kinds(&trail),
        [
            "search", "search", "ask", "open", "ask", "ask", "ask", "search", "open", "open"
        ]
    );
    let seqs: Vec<u32> = trail.steps.iter().map(|s| s.seq).collect();
    assert_eq!(seqs, (1..=10).collect::<Vec<u32>>());
    assert_eq!(trail.trail.step_count, 10);
    assert_eq!(trail.trail.label, "comets nebulae");

    // The searches.
    let StepPayload::Search(s) = &trail.steps[0].payload else {
        panic!()
    };
    assert_eq!(s.words, "comets nebulae");
    assert_eq!(s.matched, first.matched_documents);
    assert_eq!(s.results.len(), first.hits.len().min(5));
    assert_eq!(
        trail.steps[0].at, "2026-11-11T09:01:00Z",
        "updated by the repeat"
    );
    let top = &s.results[0].anchor;
    assert_eq!(
        top.ordinal,
        Some(ordinal(&rec.lib, first.hits[0].passage.unwrap()))
    );
    assert!(top.excerpt.as_ref().unwrap().chars().count() <= 300);
    assert_eq!(trail.anchors_found(0), s.results.len());
    let StepPayload::Search(s) = &trail.steps[1].payload else {
        panic!()
    };
    assert_eq!(s.filters.source_root_id.as_deref(), Some(folder.as_str()));
    assert_eq!(s.filters.folder_name.as_deref(), Some("Books"));

    // The questions, each with how it ended and the text as delivered.
    let ask = |i: usize| match &trail.steps[i].payload {
        StepPayload::Ask(a) => a.clone(),
        other => panic!("{other:?}"),
    };
    let a = ask(2);
    assert_eq!(a.question, QUESTION);
    assert_eq!(a.ending, AskEnding::Done);
    assert_eq!(a.answer, "The starship passed comets [1] and nebulae.");
    assert_eq!(a.cited, vec![1]);
    assert_eq!(a.model.as_deref(), Some("test-chat"));
    assert_eq!(a.location, Some(ProviderLocation::ThisComputer));
    assert_eq!(a.sources.len(), done.sources.len());
    for s in &a.sources {
        // Cited sources keep the excerpt as sent; the rest are anchors.
        if s.n == 1 {
            assert!(s.cited);
            assert_eq!(
                s.anchor.excerpt.as_deref(),
                Some(done.sources[0].excerpt.as_str())
            );
        } else {
            assert!(!s.cited && s.anchor.excerpt.is_none());
        }
    }
    assert_eq!(ask(4).ending, AskEnding::Stopped);
    assert!(ask(4).answer.starts_with("word0"));
    assert!(!ask(4).answer.contains("word39"));
    assert_eq!(
        ask(5).ending,
        AskEnding::Failed {
            code: AnswerErrorCode::ServiceError
        }
    );
    assert_eq!(ask(5).answer, "The starship passed comets [1]");
    assert_eq!(ask(6).ending, AskEnding::NothingFound);
    assert!(ask(6).sources.is_empty());

    // The opens: at the passage the user saw.
    let open = |i: usize| match &trail.steps[i].payload {
        StepPayload::Open(o) => o.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(open(3).via, Reached::Citation);
    assert_eq!(
        open(3).anchor.ordinal,
        Some(ordinal(&rec.lib, cited.passage.unwrap()))
    );
    assert_eq!(open(8).via, Reached::Result);
    assert_eq!(open(8).actions, [OpenAction::Details, OpenAction::Open]);
    assert_eq!(
        open(8).anchor.ordinal,
        Some(ordinal(&rec.lib, hit.passage.unwrap()))
    );
    assert_eq!(open(9).via, Reached::Related);
    assert_eq!(open(9).anchor.ordinal, None, "the whole document");
    assert!(trail.anchors_found(8) == 1 && trail.anchors_found(9) == 1);

    // What each step touched, for "already opened" later.
    let store = rec.store();
    let touched: i64 = store
        .query_row(
            "SELECT count(*) FROM step_documents d JOIN trail_steps s ON s.id = d.step_id
             WHERE s.kind = 'open'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(touched, 3);
    // The filter finds the trail by its searches and questions.
    let text: String = store
        .query_row("SELECT search_text FROM trails", [], |r| r.get(0))
        .unwrap();
    assert!(text.contains("comets nebulae") && text.contains(QUESTION));
    // Recording wrote nothing to the catalog's jobs.
    assert_eq!(catalog_changes(&rec), jobs_before);
}

trait Found {
    fn anchors_found(&self, step: usize) -> usize;
}

impl Found for TrailDetail {
    /// How many of a step's anchors resolve to `found` now.
    fn anchors_found(&self, step: usize) -> usize {
        self.steps[step]
            .anchors
            .iter()
            .filter(|a| a.state == atlas_lib::domain::dto::AnchorState::Found && a.excerpt_found)
            .count()
    }
}

#[test]
fn p3_10_paused_writes_nothing_and_a_failing_store_never_fails_the_search() {
    let mut rec = Rec::new();
    rec.answer_with(streamed(&sse(&["Comets [1]."]), Duration::ZERO));
    let q = Lib::query("comets nebulae");

    // Paused: everything works, and nothing is written.
    rec.set_privacy(|p| p.record_trails = false);
    let status = rec.recorder().status();
    assert_eq!(status.state, atlas_lib::domain::dto::RecordingState::Paused);
    let before = rec.changes();
    let page = rec.search(&q, SearchIntent::Submit);
    assert!(!page.hits.is_empty());
    let started = rec.started(QUESTION);
    assert!(wait_until(LONG, || rec.ended(&started.answer_id)));
    rec.recorder().open(
        &page.hits[0].file_instance_id,
        &OpenVia::Result,
        OpenAction::Open,
        OpenPlace::default(),
    );
    assert!(matches!(
        rec.ask("zqxjv plorbix"),
        AskReply::NothingFound { .. }
    ));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(rec.changes(), before, "nothing was written while paused");

    // A note is the user's own act: written while paused, to an existing
    // trail only.
    assert!(rec.recorder().note("no-such-trail", None, "mine").is_err());

    // The store refuses writes: searches and answers still succeed.
    rec.set_privacy(|p| p.record_trails = true);
    rec.recorder().search(&q, SearchIntent::Submit, &page);
    assert_eq!(rec.recorded(), 1);
    rec.lib
        .root()
        .knowledge()
        .call(|c| c.pragma_update(None, "query_only", true))
        .unwrap()
        .unwrap();
    rec.statuses.lock().unwrap().clear();
    rec.later();
    let page = rec.search(&Lib::query("glowing starship"), SearchIntent::Submit);
    assert!(!page.hits.is_empty(), "the search succeeds");
    let started = rec.started(QUESTION);
    assert!(wait_until(LONG, || rec.ended(&started.answer_id)));
    assert!(matches!(
        rec.steps(&started.answer_id).last(),
        Some(AnswerStep::Done { .. })
    ));
    assert!(wait_until(LONG, || rec.recorder().status().failed));
    rec.search(&Lib::query("sourdough"), SearchIntent::Submit);
    std::thread::sleep(Duration::from_millis(300));
    let failures = rec
        .statuses
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.failed)
        .count();
    assert_eq!(failures, 1, "reported once");
    assert_eq!(rec.recorded(), 1, "nothing more was written");

    // Writable again: the next step is saved and the failure clears.
    rec.lib
        .root()
        .knowledge()
        .call(|c| c.pragma_update(None, "query_only", false))
        .unwrap()
        .unwrap();
    rec.search(&Lib::query("harbor ballads"), SearchIntent::Submit);
    let status = rec.recorder().status();
    assert!(!status.failed);
    assert_eq!(status.step_count, 2);

    // Paused again, a note the user adds is still written.
    rec.set_privacy(|p| p.record_trails = false);
    let trail = status.current_trail_id.unwrap();
    rec.recorder()
        .note(&trail, None, "read the harbor one")
        .unwrap();
    assert_eq!(rec.trail().steps.len(), 3);
}
