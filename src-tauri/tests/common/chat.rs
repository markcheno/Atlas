//! A chat role on a scripted service whose replies can change per question,
//! and asking through the answer service as `ask_atlas` does, with the
//! library's recorder (Slices 3.3, 3.4).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_lib::domain::dto::{
    AnswerEvent, AnswerStep, AskQuery, AskReply, AskStarted, FileType, ProviderKind,
    ProviderProfileInput, RecordId,
};
use atlas_lib::services::answers::stream::ChatLimits;
use atlas_lib::services::answers::{self, AnswerEvents, AskDeps};
use atlas_lib::services::providers::{privacy, profiles, url_policy};
use atlas_lib::services::search::MeaningContext;
use serde_json::json;

use super::http::{MockServer, Reply, stream};
use super::meaning::Lib;

pub const SSE: &str = "text/event-stream";

pub fn delta(text: &str) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices":[{"delta":{"content": text}}]})
    )
}

/// `parts` as a finished OpenAI-style stream.
pub fn sse(parts: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = parts.iter().map(|p| delta(p)).collect();
    out.push("data: [DONE]\n\n".into());
    out
}

pub fn streamed(pieces: &[String], pause: Duration) -> Reply {
    let refs: Vec<&str> = pieces.iter().map(String::as_str).collect();
    stream(SSE, &refs, pause)
}

/// A slow answer of `words` pieces, to stop partway.
pub fn slow(words: usize) -> Reply {
    let parts: Vec<String> = (0..words).map(|i| format!("word{i} ")).collect();
    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    streamed(&sse(&refs), Duration::from_millis(100))
}

/// The chat role, on a service each `answer_with` replaces.
pub struct Chat {
    profile: String,
    servers: Mutex<Vec<MockServer>>,
    pub events: Arc<Mutex<Vec<AnswerEvent>>>,
}

impl Chat {
    /// Saves a chat service and makes it the chat role.
    pub fn new(lib: &Lib) -> Self {
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
            profile: saved.id,
            servers: Mutex::default(),
            events: Arc::default(),
        }
    }

    /// The next questions are answered with `reply`.
    pub fn answer_with(&self, lib: &Lib, reply: Reply) {
        let server = MockServer::start(vec![("POST /v1/chat/completions", reply)]);
        let (id, url) = (self.profile.clone(), server.url("/v1"));
        lib.root()
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

    /// What `ask_atlas` does, with the library's recorder.
    pub fn ask_in(
        &self,
        lib: &Lib,
        question: &str,
        source_root_id: Option<RecordId>,
        file_type: Option<FileType>,
    ) -> AskReply {
        let root = lib.root();
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
                    embedder: &lib.embedder,
                    vault: &lib.vault,
                    allow_remote: privacy::remote_embedding(&settings),
                },
                vault: &lib.vault,
                privacy: &settings,
                locate: url_policy::location_of,
                limits: ChatLimits::default(),
                events,
                answers: root.answers(),
                recorder: Some(root.recorder()),
                diagnostics: None,
            },
            &AskQuery {
                question: question.into(),
                source_root_id,
                file_type,
            },
        )
        .unwrap()
    }

    pub fn ask(&self, lib: &Lib, question: &str) -> AskReply {
        self.ask_in(lib, question, None, None)
    }

    pub fn started(&self, lib: &Lib, question: &str) -> AskStarted {
        match self.ask(lib, question) {
            AskReply::Started(s) => s,
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    pub fn steps(&self, id: &str) -> Vec<AnswerStep> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.answer_id == id)
            .map(|e| e.step.clone())
            .collect()
    }

    pub fn ended(&self, id: &str) -> bool {
        self.steps(id).iter().any(|s| {
            matches!(
                s,
                AnswerStep::Done { .. } | AnswerStep::Stopped | AnswerStep::Error { .. }
            )
        })
    }
}
