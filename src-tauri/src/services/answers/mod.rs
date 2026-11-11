//! Ask Atlas (phase2-spec §5.5, §7.6; ADR 0015): an answer to a question
//! from the library, with citations checked against what was sent.
//!
//! 1. **Retrieve** with hybrid search: the best documents' passages as
//!    numbered excerpts ([`retrieve`]). Nothing found: the model is not
//!    asked.
//! 2. **Disclose**: an internet service is sent nothing until the user
//!    agreed, for that profile at that address, having seen the sizes.
//!    "Disable all external AI" refuses it outright.
//! 3. **Prompt**: fixed instructions, then the excerpts as delimited data,
//!    then the question ([`prompt`]).
//! 4. **Stream** the chat role's answer ([`stream`]), checking each `[n]`
//!    as it arrives ([`cite`]); `answer-stream` events carry it.
//!
//! One answer at a time per library: a new question, Stop, or the library
//! closing ends the one before. An answer is kept only as a step of the
//! user's trail, in the knowledge store, while recording is on (Slice 3.3,
//! ADR 0017, amending ADR 0015): never in the catalog, never in a file of
//! its own. Atlas keeps no log.

pub mod cite;
pub mod prompt;
pub mod retrieve;
pub mod stream;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusqlite::{Connection, OptionalExtension};

use crate::domain::dto::{
    AnswerErrorCode, AnswerEvent, AnswerStep, AskEnding, AskQuery, AskReply, AskStarted,
    DisclosureNeeded, PrivacySettings, ProviderLocation, SearchQuery,
};
use crate::error::{AtlasError, AtlasResult};
use crate::services::catalog::reader::CatalogReader;
use crate::services::knowledge::Recorder;
use crate::services::knowledge::record::{AnswerOutcome, AskStart};
use crate::services::providers::privacy::{self, Use};
use crate::services::providers::profiles::{self, Prepared};
use crate::services::providers::secrets::SecretVault;
use crate::services::search::{self, MeaningContext};
use cite::{Citations, Out};
use stream::{ChatLimits, ChatTarget, Ended, Piece};

/// The longest question, in characters.
pub const MAX_QUESTION_CHARS: usize = 2_000;
/// At most one text event this often; the rest waits and joins it.
const DELTA_EVERY: Duration = Duration::from_millis(50);

/// Where the answer's events go (`answer-stream` in the app).
pub type AnswerEvents = Arc<dyn Fn(&AnswerEvent) + Send + Sync>;

/// How a service's address is placed: [`url_policy::location_of`] in the
/// app; tests stand in for an internet service on this computer.
///
/// [`url_policy::location_of`]: crate::services::providers::url_policy::location_of
pub type Locate = fn(&str) -> ProviderLocation;

/// Everything an answer needs from the open library and the app.
pub struct AskDeps<'a> {
    pub reader: &'a CatalogReader,
    pub meaning: MeaningContext<'a>,
    pub vault: &'a SecretVault,
    pub privacy: &'a PrivacySettings,
    pub locate: Locate,
    pub limits: ChatLimits,
    pub events: AnswerEvents,
    pub answers: &'a Arc<AnswerRegistry>,
    /// Records the question into the user's trail (Slice 3.3); `None`
    /// records nothing.
    pub recorder: Option<&'a Arc<Recorder>>,
}

/// The chat role's service, as stored.
struct ChatRole {
    profile_id: String,
    label: String,
    base_url: String,
    model: String,
}

fn chat_role(conn: &Connection) -> rusqlite::Result<Option<ChatRole>> {
    conn.query_row(
        "SELECT p.id, p.label, p.base_url, r.model_id FROM model_roles r
         JOIN provider_profiles p ON p.id = r.provider_profile_id
         WHERE r.role = 'chat'",
        [],
        |r| {
            Ok(ChatRole {
                profile_id: r.get(0)?,
                label: r.get(1)?,
                base_url: r.get(2)?,
                model: r.get(3)?,
            })
        },
    )
    .optional()
}

/// Answers `query` from the library: the sources and a streamed answer,
/// or why not yet. Blocks only for retrieval; the answer streams on its
/// own thread.
pub fn ask(deps: &AskDeps<'_>, query: &AskQuery) -> AtlasResult<AskReply> {
    let question = query.question.trim().to_owned();
    if question.is_empty() || question.chars().count() > MAX_QUESTION_CHARS {
        return Err(AtlasError::InvalidQuestion {
            max: MAX_QUESTION_CHARS,
        });
    }
    let search_query = SearchQuery {
        text: question.clone(),
        source_root_id: query.source_root_id.clone(),
        file_type: query.file_type,
        include_missing: false,
        page: 0,
        page_size: retrieve::DOCUMENTS as u32,
    };
    let (role, prepared, retrieval, sources) = deps
        .reader
        .read(|c| {
            let Some(role) = chat_role(c)? else {
                return Ok(None);
            };
            if !privacy::allows(deps.privacy, (deps.locate)(&role.base_url), Use::Ask) {
                return Ok(Some((role, None, None, Vec::new())));
            }
            let prepared = profiles::resolve_saved(c, deps.vault, &role.profile_id);
            let retrieval = search::retrieve(c, &deps.meaning, &search_query, retrieve::DOCUMENTS)?;
            let sources = retrieve::excerpts(c, &retrieval.found)?;
            Ok::<_, rusqlite::Error>(Some((role, Some(prepared), Some(retrieval), sources)))
        })?
        .map_err(AtlasError::db("find passages for the question"))?
        .ok_or(AtlasError::ChatModelNeeded)?;
    let (Some(prepared), Some(retrieval)) = (prepared, retrieval) else {
        return Err(AtlasError::ExternalAiDisabled);
    };
    let Prepared::Ready {
        kind,
        endpoint,
        key,
    } = prepared?
    else {
        return Err(AtlasError::InvalidUrl);
    };

    let location = (deps.locate)(&role.base_url);
    if sources.is_empty() {
        if let Some(recorder) = deps.recorder {
            recorder.nothing_found(query, retrieval.semantic, &role.model, location);
        }
        return Ok(AskReply::NothingFound {
            semantic: retrieval.semantic,
        });
    }
    if location == ProviderLocation::Internet
        && !privacy::acknowledged(deps.privacy, &role.profile_id, &role.base_url)
    {
        let bytes = question.len() + sources.iter().map(|s| s.excerpt.len()).sum::<usize>();
        return Ok(AskReply::NeedsDisclosure(DisclosureNeeded {
            profile_id: role.profile_id,
            profile_label: role.label,
            base_url: role.base_url,
            excerpt_count: sources.len() as u32,
            bytes: bytes as u64,
        }));
    }

    let blocks: Vec<prompt::Block<'_>> = sources
        .iter()
        .map(|s| prompt::Block {
            n: s.n,
            title: &s.title,
            location: s.location.as_deref(),
            text: &s.excerpt,
        })
        .collect();
    let messages = prompt::messages(&blocks, &question);
    let model = role.model.clone();
    let target = ChatTarget {
        kind,
        base: endpoint.base,
        key,
        model: role.model,
        nearby: location != ProviderLocation::Internet,
    };
    let answer_id = uuid::Uuid::new_v4().to_string();
    let weak = retrieve::weak(
        &retrieval.found,
        sources.len(),
        retrieval.best_similarity,
        retrieval.min_similarity,
        retrieval.weak_margin,
    );
    let count = sources.len() as u32;
    // What the trail step will need, taken now: the thread has only the
    // prompt.
    let record = deps.recorder.and_then(|recorder| {
        recorder
            .ask_started(AskStart {
                answer_id: &answer_id,
                query,
                sources: &sources,
                weak,
                semantic: retrieval.semantic,
                model: &model,
                location,
            })
            .map(|record| (Arc::clone(recorder), record))
    });
    let (events, limits, id) = (Arc::clone(&deps.events), deps.limits, answer_id.clone());
    deps.answers.start(answer_id.clone(), move |stop| {
        let outcome = answer(&id, &target, limits, &messages, count, &events, &stop);
        // After the last event, so the answer never waits on the store.
        if let Some((recorder, record)) = record {
            recorder.ask(record, outcome);
        }
    });
    Ok(AskReply::Started(AskStarted {
        answer_id,
        sources,
        weak,
        semantic: retrieval.semantic,
    }))
}

/// Streams one answer, checking its citations, and reports each step.
/// Returns the answer as delivered, for the trail.
fn answer(
    id: &str,
    target: &ChatTarget,
    limits: ChatLimits,
    messages: &serde_json::Value,
    sources: u32,
    events: &AnswerEvents,
    stop: &AtomicBool,
) -> AnswerOutcome {
    let emit = |step: AnswerStep| {
        events(&AnswerEvent {
            answer_id: id.to_owned(),
            step,
        });
    };
    let mut cite = Citations::new(sources);
    let mut held = String::new();
    // Everything sent to the screen: the text, its `[n]` markers in it.
    let mut delivered = String::new();
    let mut said_thinking = false;
    let mut any_text = false;
    let mut last = Instant::now();
    let mut flush = |held: &mut String, outs: Vec<Out>, now: bool| {
        for out in outs {
            match out {
                Out::Text(t) => {
                    delivered.push_str(&t);
                    held.push_str(&t);
                }
                Out::Cited(n) => {
                    if !held.is_empty() {
                        emit(AnswerStep::Delta {
                            text: std::mem::take(held),
                        });
                    }
                    emit(AnswerStep::Citation { n });
                }
            }
        }
        if !held.is_empty() && (now || last.elapsed() >= DELTA_EVERY) {
            emit(AnswerStep::Delta {
                text: std::mem::take(held),
            });
            last = Instant::now();
        }
    };
    let stopped = || stop.load(Ordering::SeqCst);
    let ended = stream::run(target, limits, messages, &stopped, |piece| match piece {
        Piece::Thinking => {
            if !said_thinking && !any_text {
                said_thinking = true;
                emit(AnswerStep::Thinking);
            }
        }
        Piece::Text(t) => {
            any_text |= !t.trim().is_empty();
            let outs = cite.feed(&t);
            flush(&mut held, outs, false);
        }
        Piece::Quiet => flush(&mut held, Vec::new(), false),
    });
    let ending = match ended {
        Ended::Stopped => {
            emit(AnswerStep::Stopped);
            AskEnding::Stopped
        }
        // A thinking model can spend its whole allowance reasoning: that
        // is no answer, not an empty one.
        Ended::Finished if !any_text => {
            emit(AnswerStep::Error {
                code: AnswerErrorCode::NoAnswer,
                message: if said_thinking {
                    "The model spent its whole allowance thinking and wrote no answer."
                } else {
                    "The model finished without writing an answer."
                }
                .to_owned(),
                partial: false,
                cited: Vec::new(),
                dropped: 0,
            });
            AskEnding::Failed {
                code: AnswerErrorCode::NoAnswer,
            }
        }
        Ended::Finished => {
            let outs = cite.finish();
            flush(&mut held, outs, true);
            emit(AnswerStep::Done {
                cited: cite.cited().to_vec(),
                dropped: cite.dropped(),
            });
            AskEnding::Done
        }
        Ended::Failed { code, message } => {
            let outs = cite.finish();
            flush(&mut held, outs, true);
            emit(AnswerStep::Error {
                code,
                message,
                partial: any_text,
                cited: cite.cited().to_vec(),
                dropped: cite.dropped(),
            });
            AskEnding::Failed { code }
        }
    };
    AnswerOutcome {
        text: delivered,
        ending,
        cited: cite.cited().to_vec(),
        dropped: cite.dropped(),
    }
}

/// The library's one answer in progress.
#[derive(Default)]
pub struct AnswerRegistry {
    active: Mutex<Option<Active>>,
}

struct Active {
    id: String,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl AnswerRegistry {
    /// Starts `work` on its own thread, ending the answer before it.
    fn start(&self, id: String, work: impl FnOnce(Arc<AtomicBool>) + Send + 'static) {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let previous = {
            let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
            let thread = std::thread::Builder::new()
                .name("atlas-answer".into())
                .spawn(move || work(flag))
                .expect("an answer thread can be started");
            active.replace(Active { id, stop, thread })
        };
        if let Some(previous) = previous {
            previous.stop.store(true, Ordering::SeqCst);
        }
    }

    /// Stops the answer `id` if it is still the one in progress; `false`
    /// when there is nothing to stop.
    pub fn cancel(&self, id: &str) -> bool {
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        match active.as_ref() {
            Some(a) if a.id == id && !a.thread.is_finished() => {
                a.stop.store(true, Ordering::SeqCst);
                true
            }
            _ => false,
        }
    }

    /// Whether an answer is still streaming.
    pub fn busy(&self) -> bool {
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        active.as_ref().is_some_and(|a| !a.thread.is_finished())
    }
}

impl Drop for AnswerRegistry {
    /// The library is closing: end the answer and wait for its thread,
    /// which stops within a poll.
    fn drop(&mut self) {
        let active = self.active.get_mut().unwrap_or_else(|e| e.into_inner());
        if let Some(a) = active.take() {
            a.stop.store(true, Ordering::SeqCst);
            let _ = a.thread.join();
        }
    }
}
