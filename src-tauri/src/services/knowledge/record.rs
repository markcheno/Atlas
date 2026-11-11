//! Recording trails (phase3-spec §7.1, ADR 0017): searches, questions and
//! the sources opened from them become steps of the current trail.
//!
//! - **Rust records from its own state.** The renderer says only that a
//!   search was submitted, or which source it opened and how it got
//!   there; the words, the answer and the anchors come from here.
//! - **Paused, nothing is written** (`privacy.recordTrails` off). A note
//!   the user adds is their own act and is written anyway.
//! - **A recording failure never fails the search or the answer.** It is
//!   shown once (`RecordingStatus.failed`) until a step is saved again.
//!   Atlas has no log, so it is written nowhere.
//! - **Nothing here keeps content outside the store.** The places of the
//!   last results and the last answer's sources are kept in memory, so an
//!   open from them anchors at the passage the user saw. So is the last
//!   ended answer, recording or not, so it can be saved as a finding
//!   (Slice 3.5) until the next question.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension};

use super::anchor::{self, Place};
use super::store::Inner;
use super::trails::{self, Appended, PAYLOAD_VERSION, Target};
use crate::domain::dto::{
    Anchor, AskEnding, AskQuery, AskStep, FilePlace, FileType, KnowledgeState, NoteStep,
    OpenAction, OpenPlace, OpenStep, OpenVia, ProviderLocation, Reached, RecordId, RecordingState,
    RecordingStatus, SearchIntent, SearchPage, SearchQuery, SearchStep, SemanticStatus, SourceUsed,
    StepFilters, StepPayload, StepResult, StepSource, TrailDetail, TrailPage, TrailQuery,
    TrailSummary,
};
use crate::error::{AtlasError, AtlasResult};
use crate::services::catalog::reader::CatalogReader;

/// Where the recording status goes (`trail-changed` in the app).
pub type TrailEvents = Arc<dyn Fn(&RecordingStatus) + Send + Sync>;

/// A search step keeps this many of its results.
pub const SEARCH_RESULTS: usize = 5;
/// A search result's excerpt, enough to find it again after a re-read.
pub const RESULT_EXCERPT_CHARS: usize = 300;

/// What a question's step needs that the answer's thread doesn't have:
/// taken when the answer starts, recorded when it ends.
#[derive(Debug, Clone)]
pub struct AskRecord {
    /// Empty for a question with nothing found (no answer started).
    pub(crate) answer_id: String,
    pub(crate) question: String,
    filters: StepFilters,
    /// Each source's number and anchor, its excerpt as sent.
    pub(crate) sources: Vec<(u32, Anchor)>,
    weak: bool,
    semantic: SemanticStatus,
    model: Option<String>,
    location: Option<ProviderLocation>,
}

/// The last answer that ended, kept in memory until the next question.
#[derive(Debug, Clone)]
pub struct LastAnswer {
    pub record: AskRecord,
    pub outcome: AnswerOutcome,
    /// Its trail step, when it was recorded.
    pub step_id: Option<String>,
}

/// How an answer ended, from its thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerOutcome {
    /// The text as delivered, its checked `[n]` markers in it.
    pub text: String,
    pub ending: AskEnding,
    pub cited: Vec<u32>,
    pub dropped: u32,
}

/// A source of the answer on screen: its number, file and passage.
struct SourcePlace {
    n: u32,
    file: RecordId,
    passage: Option<i64>,
}

/// The places behind what is on screen now: the last page of results and
/// the last answer's sources. In memory only.
#[derive(Default)]
struct Places {
    /// File id to the passage its hit showed.
    results: HashMap<RecordId, i64>,
    /// The answer id, and its sources.
    answer: Option<(String, Vec<SourcePlace>)>,
}

/// An answer that has just started, as `ask` knows it.
pub struct AskStart<'a> {
    pub answer_id: &'a str,
    pub query: &'a AskQuery,
    pub sources: &'a [SourceUsed],
    pub weak: bool,
    pub semantic: SemanticStatus,
    pub model: &'a str,
    pub location: ProviderLocation,
}

/// Records steps into the library's knowledge store; one per open
/// library.
pub struct Recorder {
    store: Arc<Inner>,
    catalog: Arc<CatalogReader>,
    events: Option<TrailEvents>,
    failed: AtomicBool,
    places: Mutex<Places>,
    last: Mutex<Option<LastAnswer>>,
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => text[..at].to_owned(),
        None => text.to_owned(),
    }
}

/// The filters with the folder's name as it is now (it is recorded as it
/// was).
fn filters(
    conn: &Connection,
    source_root_id: Option<&RecordId>,
    file_type: Option<FileType>,
) -> rusqlite::Result<StepFilters> {
    let folder_name = match source_root_id {
        Some(id) => conn
            .query_row(
                "SELECT display_path FROM source_roots WHERE id = ?1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|path| {
                Path::new(&path)
                    .file_name()
                    .map_or(path.clone(), |n| n.to_string_lossy().into_owned())
            }),
        None => None,
    };
    Ok(StepFilters {
        source_root_id: source_root_id.cloned(),
        folder_name,
        file_type,
    })
}

impl Recorder {
    pub fn new(
        store: Arc<Inner>,
        catalog: Arc<CatalogReader>,
        events: Option<TrailEvents>,
    ) -> Self {
        Self {
            store,
            catalog,
            events,
            failed: AtomicBool::new(false),
            places: Mutex::new(Places::default()),
            last: Mutex::new(None),
        }
    }

    fn ready(&self) -> AtlasResult<()> {
        if self.store.state() == KnowledgeState::Ready {
            Ok(())
        } else {
            Err(AtlasError::Knowledge("reach"))
        }
    }

    /// Whether a step would be written now.
    pub fn recording(&self) -> bool {
        self.store.state() == KnowledgeState::Ready && self.store.trail_settings().recording()
    }

    fn places(&self) -> std::sync::MutexGuard<'_, Places> {
        self.places.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The indicator's view.
    pub fn status(&self) -> RecordingStatus {
        let failed = self.failed.load(Ordering::SeqCst);
        if self.store.state() != KnowledgeState::Ready {
            return RecordingStatus {
                state: RecordingState::Unavailable,
                current_trail_id: None,
                step_count: 0,
                first_step_note: false,
                failed,
            };
        }
        let now = self.store.clock().now();
        let (current, note) = self
            .store
            .read(|c| {
                Ok::<_, rusqlite::Error>((
                    trails::current_steps(c, now)?,
                    trails::first_step_note(c)?,
                ))
            })
            .ok()
            .and_then(Result::ok)
            .unwrap_or((None, false));
        RecordingStatus {
            state: if self.store.trail_settings().recording() {
                RecordingState::Recording
            } else {
                RecordingState::Paused
            },
            current_trail_id: current.as_ref().map(|(id, _)| id.clone()),
            step_count: current.map_or(0, |(_, n)| n),
            first_step_note: note,
            failed,
        }
    }

    /// Sends the status to the screen (after a step, a change, a setting).
    pub fn emit(&self) {
        if let Some(events) = &self.events {
            events(&self.status());
        }
    }

    /// Writes one step in one transaction. A failure is reported once and
    /// goes no further.
    fn write(&self, target: Target, payload: StepPayload) -> Option<Appended> {
        let now = self.store.clock().now();
        let written = self.store.call(move |c| {
            let tx = c.transaction()?;
            let appended = trails::append(&tx, now, &target, &payload)?;
            tx.commit()?;
            Ok::<_, rusqlite::Error>(appended)
        });
        match written {
            Ok(Ok(appended)) => {
                self.failed.store(false, Ordering::SeqCst);
                self.emit();
                Some(appended)
            }
            _ => {
                if !self.failed.swap(true, Ordering::SeqCst) {
                    self.emit();
                }
                None
            }
        }
    }

    // ------------------------------------------------------------ steps

    /// After `search_documents`: remembers the places shown, and records a
    /// submitted search.
    pub fn search(&self, query: &SearchQuery, intent: SearchIntent, page: &SearchPage) {
        self.places().results = page
            .hits
            .iter()
            .filter_map(|h| Some((h.file_instance_id.clone(), h.passage?)))
            .collect();
        let words = query.text.trim();
        if intent != SearchIntent::Submit || words.is_empty() || !self.recording() {
            return;
        }
        let built = self.catalog.read(|c| -> AtlasResult<SearchStep> {
            let mut results = Vec::new();
            for hit in page.hits.iter().take(SEARCH_RESULTS) {
                let place = hit.passage.map_or(Place::Document, Place::Passage);
                let Some(mut anchor) = anchor::build(c, &hit.file_instance_id, place)? else {
                    continue;
                };
                anchor.excerpt = anchor.excerpt.map(|e| truncate(&e, RESULT_EXCERPT_CHARS));
                results.push(StepResult {
                    anchor,
                    why: hit.why,
                });
            }
            Ok(SearchStep {
                v: PAYLOAD_VERSION,
                words: words.to_owned(),
                filters: filters(c, query.source_root_id.as_ref(), query.file_type)
                    .map_err(AtlasError::db("be read"))?,
                matched: page.matched_documents,
                results,
                semantic: page.semantic,
            })
        });
        match built {
            Ok(Ok(step)) => {
                self.write(Target::Current, StepPayload::Search(step));
            }
            _ => self.report_failure(),
        }
    }

    fn report_failure(&self) {
        if !self.failed.swap(true, Ordering::SeqCst) {
            self.emit();
        }
    }

    /// When an answer starts: remembers its sources' places, and what its
    /// step, or a finding saved from it, will need.
    pub fn ask_started(&self, start: AskStart<'_>) -> Option<AskRecord> {
        let AskStart {
            answer_id,
            query,
            sources,
            weak,
            semantic,
            model,
            location,
        } = start;
        self.places().answer = Some((
            answer_id.to_owned(),
            sources
                .iter()
                .map(|s| SourcePlace {
                    n: s.n,
                    file: s.file_instance_id.clone(),
                    passage: s.passage,
                })
                .collect(),
        ));
        // A newer question: the one before can no longer be saved.
        *self.last.lock().unwrap_or_else(|p| p.into_inner()) = None;
        let built = self.catalog.read(|c| -> AtlasResult<AskRecord> {
            let mut anchors = Vec::new();
            for s in sources {
                let place = s.passage.map_or(Place::Document, Place::Passage);
                let Some(mut anchor) = anchor::build(c, &s.file_instance_id, place)? else {
                    continue;
                };
                // The excerpt as it was sent, not the whole passage.
                anchor.excerpt = Some(truncate(&s.excerpt, anchor::ANCHOR_EXCERPT_CHARS));
                anchors.push((s.n, anchor));
            }
            Ok(AskRecord {
                answer_id: answer_id.to_owned(),
                question: query.question.trim().to_owned(),
                filters: filters(c, query.source_root_id.as_ref(), query.file_type)
                    .map_err(AtlasError::db("be read"))?,
                sources: anchors,
                weak,
                semantic,
                model: Some(model.to_owned()),
                location: Some(location),
            })
        });
        match built {
            Ok(Ok(record)) => Some(record),
            _ => {
                if self.recording() {
                    self.report_failure();
                }
                None
            }
        }
    }

    /// When an answer has ended, however it ended: kept for a finding, and
    /// recorded while recording is on.
    pub fn ask(&self, record: AskRecord, outcome: AnswerOutcome) {
        let keep = !record.answer_id.is_empty();
        if keep {
            *self.last.lock().unwrap_or_else(|p| p.into_inner()) = Some(LastAnswer {
                record: record.clone(),
                outcome: outcome.clone(),
                step_id: None,
            });
        }
        if !self.recording() {
            return;
        }
        let answer_id = record.answer_id.clone();
        let sources = record
            .sources
            .into_iter()
            .map(|(n, mut anchor)| {
                let cited = outcome.cited.contains(&n);
                if !cited {
                    anchor.excerpt = None;
                }
                StepSource { n, cited, anchor }
            })
            .collect();
        let written = self.write(
            Target::Current,
            StepPayload::Ask(AskStep {
                v: PAYLOAD_VERSION,
                question: record.question,
                filters: record.filters,
                answer: outcome.text,
                ending: outcome.ending,
                cited: outcome.cited,
                dropped: outcome.dropped,
                weak: record.weak,
                semantic: record.semantic,
                model: record.model,
                location: record.location,
                sources,
            }),
        );
        if let Some(appended) = written {
            let mut last = self.last.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(l) = last
                .as_mut()
                .filter(|l| keep && l.record.answer_id == answer_id)
            {
                l.step_id = Some(appended.step_id);
            }
        }
    }

    /// A question with nothing in the library to answer it from.
    pub fn nothing_found(
        &self,
        query: &AskQuery,
        semantic: SemanticStatus,
        model: &str,
        location: ProviderLocation,
    ) {
        if !self.recording() {
            return;
        }
        let Some(record) = self
            .catalog
            .read(|c| filters(c, query.source_root_id.as_ref(), query.file_type))
            .ok()
            .and_then(Result::ok)
            .map(|filters| AskRecord {
                answer_id: String::new(),
                question: query.question.trim().to_owned(),
                filters,
                sources: Vec::new(),
                weak: true,
                semantic,
                model: Some(model.to_owned()),
                location: Some(location),
            })
        else {
            return self.report_failure();
        };
        self.ask(
            record,
            AnswerOutcome {
                text: String::new(),
                ending: AskEnding::NothingFound,
                cited: Vec::new(),
                dropped: 0,
            },
        );
    }

    /// A source the user opened. From the Library it is not a step.
    pub fn open(&self, file_id: &str, via: &OpenVia, action: OpenAction, place: OpenPlace) {
        let reached = match via {
            OpenVia::Library => return,
            OpenVia::Result => Reached::Result,
            OpenVia::Citation { .. } => Reached::Citation,
            OpenVia::Related => Reached::Related,
            OpenVia::Suggestion => Reached::Suggestion,
            OpenVia::Bookmark => Reached::Bookmark,
            OpenVia::Trail => Reached::Trail,
        };
        if !self.recording() {
            return;
        }
        let passage = {
            let places = self.places();
            match via {
                OpenVia::Result => places.results.get(file_id).copied(),
                OpenVia::Citation { answer_id, n } => places
                    .answer
                    .as_ref()
                    .filter(|(id, _)| id == answer_id)
                    .and_then(|(_, sources)| {
                        sources
                            .iter()
                            .find(|s| s.n == *n && s.file == file_id)
                            .and_then(|s| s.passage)
                    }),
                _ => None,
            }
        };
        let place = passage
            .map(Place::Passage)
            .or(place.page_no.map(Place::Page))
            .unwrap_or(Place::Document);
        let built = self.catalog.read(|c| -> AtlasResult<Option<Anchor>> {
            // A place that has gone (a re-read since) falls back to the
            // whole document.
            match anchor::build(c, file_id, place)? {
                Some(a) => Ok(Some(a)),
                None if place != Place::Document => anchor::build(c, file_id, Place::Document),
                None => Ok(None),
            }
        });
        match built {
            // A file Atlas hasn't read has no document to anchor to.
            Ok(Ok(None)) => {}
            Ok(Ok(Some(anchor))) => {
                self.write(
                    Target::Current,
                    StepPayload::Open(OpenStep {
                        v: PAYLOAD_VERSION,
                        anchor,
                        via: reached,
                        actions: vec![action],
                    }),
                );
            }
            _ => self.report_failure(),
        }
    }

    // ------------------------------------------------------- for saved

    pub(crate) fn store(&self) -> &Arc<Inner> {
        &self.store
    }

    pub(crate) fn catalog(&self) -> &Arc<CatalogReader> {
        &self.catalog
    }

    /// The last ended answer, if it is `answer_id`.
    pub(crate) fn last_answer(&self, answer_id: &str) -> Option<LastAnswer> {
        self.last
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .filter(|l| l.record.answer_id == answer_id)
    }

    /// The passage behind what the screen showed of `file_id`: its hit in
    /// the last results, or source `n` of the answer on screen.
    pub(crate) fn place_for(&self, file_id: &str, place: &FilePlace) -> Place {
        let places = self.places();
        let passage = match place {
            FilePlace::Result => places.results.get(file_id).copied(),
            FilePlace::Citation { answer_id, n } => places
                .answer
                .as_ref()
                .filter(|(id, _)| id == answer_id)
                .and_then(|(_, sources)| {
                    sources
                        .iter()
                        .find(|s| s.n == *n && s.file == file_id)
                        .and_then(|s| s.passage)
                }),
            FilePlace::Page { page_no } => return Place::Page(*page_no),
            FilePlace::Document => return Place::Document,
        };
        passage.map_or(Place::Document, Place::Passage)
    }

    // ------------------------------------------------------ the user's

    /// A note: on `step_id`, or as a step of its own at the end of the
    /// trail. Written while paused too: it is the user's own act.
    pub fn note(&self, trail_id: &str, step_id: Option<&str>, text: &str) -> AtlasResult<()> {
        self.ready()?;
        let note = trails::checked_note(text)?;
        let now = self.store.clock().now();
        let (trail_id, step_id) = (trail_id.to_owned(), step_id.map(str::to_owned));
        let found = self
            .store
            .call(move |c| -> rusqlite::Result<bool> {
                let tx = c.transaction()?;
                let found = match (&step_id, &note) {
                    (Some(step), note) => {
                        let belongs = tx
                            .query_row(
                                "SELECT 1 FROM trail_steps WHERE id = ?1 AND trail_id = ?2",
                                [step, &trail_id],
                                |_| Ok(()),
                            )
                            .optional()?
                            .is_some();
                        belongs && trails::set_step_note(&tx, step, note.as_deref())?
                    }
                    (None, None) => true,
                    (None, Some(text)) => {
                        match trails::append(
                            &tx,
                            now,
                            &Target::Trail(trail_id.clone()),
                            &StepPayload::Note(NoteStep {
                                v: PAYLOAD_VERSION,
                                text: text.clone(),
                            }),
                        ) {
                            Ok(_) => true,
                            Err(rusqlite::Error::QueryReturnedNoRows) => false,
                            Err(e) => return Err(e),
                        }
                    }
                };
                tx.commit()?;
                Ok(found)
            })?
            .map_err(AtlasError::db("save the note"))?;
        if !found {
            return Err(AtlasError::TrailNotFound);
        }
        self.emit();
        Ok(())
    }

    pub fn list(&self, query: &TrailQuery) -> AtlasResult<TrailPage> {
        self.store
            .read(|c| trails::list(c, query))?
            .map_err(AtlasError::db("be read"))
    }

    pub fn get(&self, trail_id: &str) -> AtlasResult<TrailDetail> {
        self.store
            .read(|k| self.catalog.read(|c| trails::get(k, c, trail_id)))???
            .ok_or(AtlasError::TrailNotFound)
    }

    /// "New trail": the next step starts one.
    pub fn start(&self) -> AtlasResult<RecordingStatus> {
        self.store
            .call(|c| trails::start(c))?
            .map_err(AtlasError::db("start a trail"))?;
        self.emit();
        Ok(self.status())
    }

    pub fn continue_trail(&self, trail_id: &str) -> AtlasResult<TrailSummary> {
        let now = self.store.clock().now();
        let id = trail_id.to_owned();
        let summary = self
            .store
            .call(move |c| trails::continue_trail(c, &id, now))?
            .map_err(AtlasError::db("continue the trail"))?
            .ok_or(AtlasError::TrailNotFound)?;
        self.emit();
        Ok(summary)
    }

    pub fn rename(&self, trail_id: &str, title: &str) -> AtlasResult<TrailSummary> {
        let title = trails::checked_title(title)?;
        let id = trail_id.to_owned();
        let summary = self
            .store
            .call(move |c| -> rusqlite::Result<Option<TrailSummary>> {
                let tx = c.transaction()?;
                if !trails::rename(&tx, &id, title.as_deref())? {
                    return Ok(None);
                }
                tx.commit()?;
                trails::summary(c, &id)
            })?
            .map_err(AtlasError::db("rename the trail"))?
            .ok_or(AtlasError::TrailNotFound)?;
        Ok(summary)
    }

    pub fn pin(&self, trail_id: &str, pinned: bool) -> AtlasResult<TrailSummary> {
        let id = trail_id.to_owned();
        self.store
            .call(move |c| -> rusqlite::Result<Option<TrailSummary>> {
                if !trails::pin(c, &id, pinned)? {
                    return Ok(None);
                }
                trails::summary(c, &id)
            })?
            .map_err(AtlasError::db("pin the trail"))?
            .ok_or(AtlasError::TrailNotFound)
    }

    pub fn delete(&self, trail_id: &str) -> AtlasResult<()> {
        let id = trail_id.to_owned();
        self.change(move |tx| trails::delete(tx, &id), "delete the trail")
    }

    pub fn delete_step(&self, step_id: &str) -> AtlasResult<()> {
        let id = step_id.to_owned();
        self.change(move |tx| trails::delete_step(tx, &id), "delete the step")
    }

    /// Every trail; bookmarks and findings are kept. The number deleted.
    pub fn delete_all(&self) -> AtlasResult<u32> {
        let n = self
            .store
            .call(|c| -> rusqlite::Result<u32> {
                let tx = c.transaction()?;
                let n = trails::delete_all(&tx)?;
                tx.commit()?;
                Ok(n)
            })?
            .map_err(AtlasError::db("delete your trails"))?;
        self.emit();
        Ok(n)
    }

    pub fn dismiss_note(&self) -> AtlasResult<RecordingStatus> {
        self.store
            .call(|c| trails::dismiss_first_step_note(c))?
            .map_err(AtlasError::db("save that"))?;
        self.emit();
        Ok(self.status())
    }

    /// One transaction that reports whether its row was there.
    fn change(
        &self,
        f: impl FnOnce(&rusqlite::Transaction) -> rusqlite::Result<bool> + Send + 'static,
        context: &'static str,
    ) -> AtlasResult<()> {
        let found = self
            .store
            .call(move |c| -> rusqlite::Result<bool> {
                let tx = c.transaction()?;
                let found = f(&tx)?;
                tx.commit()?;
                Ok(found)
            })?
            .map_err(AtlasError::db(context))?;
        if !found {
            return Err(AtlasError::TrailNotFound);
        }
        self.emit();
        Ok(())
    }
}
