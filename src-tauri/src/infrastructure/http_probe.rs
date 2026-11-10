//! Connection probes and capability tests for AI services (phase0-spec
//! §10, phase2-spec §7.1; ADR 0007, ADR 0012).
//!
//! A connection test asks the service for its list of models: bodiless
//! `GET` requests. A capability test sends one fixed request to one model
//! ("Atlas capability test" to embed, "Reply with the word OK." to chat).
//! Neither ever carries anything from the library. The embedding job
//! (Slice 2.3, ADR 0014) sends library text to the embedding role's own
//! service through [`post_embeddings`]. Redirects are never followed and
//! system proxies are never used, so a request goes exactly where the user
//! pointed it. Timeouts and response sizes are bounded; bodies are never
//! logged.

use std::future::Future;
use std::time::{Duration, Instant};

use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde_json::{Value, json};

use crate::domain::dto::{
    AiRole, CapabilityOutcome, CapabilityResult, ConnectionOutcome, ConnectionResult, ProviderKind,
};
use crate::domain::traits::ModelProvider;
use crate::infrastructure::clock::now_rfc3339;

pub const MAX_MODELS: usize = 500;

#[derive(Debug, Clone, Copy)]
pub struct ProbeConfig {
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
    pub max_body_bytes: usize,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(3),
            total_timeout: Duration::from_secs(10),
            max_body_bytes: 1024 * 1024,
        }
    }
}

impl ProbeConfig {
    /// An embedding request from the job (phase2-spec §3): up to 16 chunks,
    /// whose answer is at most 16 vectors of a few thousand numbers.
    pub fn embedding() -> Self {
        Self {
            connect_timeout: Duration::from_secs(3),
            total_timeout: Duration::from_secs(90),
            max_body_bytes: 8 * 1024 * 1024,
        }
    }

    /// A capability test: local runtimes (oMLX, Ollama, LM Studio) load a
    /// model on its first request, which can take most of a minute for a
    /// large chat model, so the total budget is two minutes.
    pub fn capability() -> Self {
        Self {
            total_timeout: Duration::from_secs(120),
            ..Self::default()
        }
    }
}

/// The most of a streamed answer a capability test reads.
pub const CAPABILITY_STREAM_CAP: usize = 64 * 1024;
/// The most of an error body read to classify a failure.
const ERROR_BODY_CAP: usize = 16 * 1024;
/// The longest single line of a streamed answer.
const STREAM_LINE_CAP: usize = 16 * 1024;

pub fn client(config: &ProbeConfig) -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(config.connect_timeout)
        .timeout(config.total_timeout)
        .user_agent(concat!("Atlas/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("static HTTP client configuration is valid")
}

/// Result of one request, before interpretation.
pub enum Fetched {
    Ok(Value),
    /// A non-2xx answer, with the error message from its JSON body when it
    /// had one (used only to classify the failure).
    Status(StatusCode, Option<String>),
    NotJson,
    TooLarge,
    Failed(ConnectionOutcome, &'static str),
}

type Failure = (ConnectionOutcome, &'static str);

fn send_failure(e: &reqwest::Error) -> Failure {
    if e.is_timeout() {
        (
            ConnectionOutcome::Timeout,
            "The service took too long to answer.",
        )
    } else if e.is_connect() {
        (
            ConnectionOutcome::NetworkError,
            "Nothing answered at this address.",
        )
    } else {
        (
            ConnectionOutcome::NetworkError,
            "The connection failed before the service answered.",
        )
    }
}

fn read_failure(e: &reqwest::Error) -> Failure {
    if e.is_timeout() {
        (
            ConnectionOutcome::Timeout,
            "The service took too long to answer.",
        )
    } else {
        (ConnectionOutcome::NetworkError, "The connection dropped.")
    }
}

async fn send(request: RequestBuilder, bearer: Option<&str>) -> Result<Response, Failure> {
    let request = match bearer {
        Some(key) => request.bearer_auth(key),
        None => request,
    };
    request.send().await.map_err(|e| send_failure(&e))
}

enum Read {
    Body(Vec<u8>),
    TooLarge,
    Failed(Failure),
}

async fn read_capped(response: &mut Response, cap: usize) -> Read {
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > cap {
                    return Read::TooLarge;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => return Read::Body(body),
            Err(e) => return Read::Failed(read_failure(&e)),
        }
    }
}

/// The message in a JSON error body: OpenAI's `{"error": {"message"}}`,
/// Ollama's `{"error": "..."}`, FastAPI's `{"detail": ...}`, or a bare
/// `{"message"}`.
pub fn error_message(body: &Value) -> Option<String> {
    let text = |v: &Value| match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(_) | Value::Array(_) => Some(v.to_string()),
        _ => None,
    };
    let found = match body.get("error") {
        Some(Value::Object(e)) => e.get("message").and_then(text),
        Some(other) => text(other),
        None => None,
    };
    found
        .or_else(|| body.get("detail").and_then(text))
        .or_else(|| body.get("message").and_then(text))
        .map(|m| m.chars().filter(|c| !c.is_control()).take(500).collect())
}

/// A non-2xx answer: its status and, when the body is a small JSON error,
/// its message.
async fn status_of(mut response: Response) -> Fetched {
    let status = response.status();
    let message = match read_capped(&mut response, ERROR_BODY_CAP).await {
        Read::Body(body) => serde_json::from_slice::<Value>(&body)
            .ok()
            .as_ref()
            .and_then(error_message),
        _ => None,
    };
    Fetched::Status(status, message)
}

async fn json_of(mut response: Response, cap: usize) -> Fetched {
    if !response.status().is_success() {
        return status_of(response).await;
    }
    match read_capped(&mut response, cap).await {
        Read::Body(body) => serde_json::from_slice(&body).map_or(Fetched::NotJson, Fetched::Ok),
        Read::TooLarge => Fetched::TooLarge,
        Read::Failed((outcome, message)) => Fetched::Failed(outcome, message),
    }
}

pub async fn fetch_json(
    client: &Client,
    config: &ProbeConfig,
    url: &str,
    bearer: Option<&str>,
) -> Fetched {
    match send(client.get(url), bearer).await {
        Ok(response) => json_of(response, config.max_body_bytes).await,
        Err((outcome, message)) => Fetched::Failed(outcome, message),
    }
}

/// `POST` a JSON body and read a JSON answer of at most `cap` bytes.
pub async fn post_json(
    client: &Client,
    url: &str,
    bearer: Option<&str>,
    body: &Value,
    cap: usize,
) -> Fetched {
    match send(client.post(url).json(body), bearer).await {
        Ok(response) => json_of(response, cap).await,
        Err((outcome, message)) => Fetched::Failed(outcome, message),
    }
}

/// How a streamed answer ended.
pub enum Streamed {
    /// The service closed the stream.
    Done,
    /// The line handler had what it needed.
    Stopped,
    Status(StatusCode, Option<String>),
    TooLarge,
    Failed(ConnectionOutcome, &'static str),
}

/// Splits a growing buffer into complete lines (without `\r\n` or `\n`),
/// keeping the unfinished tail.
pub fn take_lines(buffer: &mut Vec<u8>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(at) = buffer.iter().position(|b| *b == b'\n') {
        let mut line: Vec<u8> = buffer.drain(..=at).collect();
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        lines.push(String::from_utf8_lossy(&line).into_owned());
    }
    lines
}

/// `POST` a JSON body and hand each line of the streamed answer (SSE or
/// NDJSON) to `on_line`, which returns `false` once it has what it needs.
/// At most `cap` bytes in all and [`STREAM_LINE_CAP`] per line are read.
pub async fn post_stream(
    client: &Client,
    url: &str,
    bearer: Option<&str>,
    body: &Value,
    cap: usize,
    mut on_line: impl FnMut(&str) -> bool,
) -> Streamed {
    let mut response = match send(client.post(url).json(body), bearer).await {
        Ok(r) => r,
        Err((outcome, message)) => return Streamed::Failed(outcome, message),
    };
    if !response.status().is_success() {
        return match status_of(response).await {
            Fetched::Status(status, message) => Streamed::Status(status, message),
            _ => Streamed::Failed(ConnectionOutcome::NetworkError, "The connection dropped."),
        };
    }
    let (mut read, mut buffer) = (0usize, Vec::new());
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                read += chunk.len();
                if read > cap {
                    return Streamed::TooLarge;
                }
                buffer.extend_from_slice(&chunk);
                for line in take_lines(&mut buffer) {
                    if !on_line(&line) {
                        return Streamed::Stopped;
                    }
                }
                if buffer.len() > STREAM_LINE_CAP {
                    return Streamed::TooLarge;
                }
            }
            Ok(None) => {
                if !buffer.is_empty() {
                    let tail = String::from_utf8_lossy(&buffer).into_owned();
                    if !on_line(&tail) {
                        return Streamed::Stopped;
                    }
                }
                return Streamed::Done;
            }
            Err(e) => {
                let (outcome, message) = read_failure(&e);
                return Streamed::Failed(outcome, message);
            }
        }
    }
}

fn result(
    outcome: ConnectionOutcome,
    models: Vec<String>,
    message: impl Into<String>,
    version: Option<String>,
) -> ConnectionResult {
    ConnectionResult {
        outcome,
        model_ids: models,
        message: message.into(),
        server_version: version,
        tested_at: now_rfc3339(),
    }
}

fn status_result(status: StatusCode, version: Option<String>, hint: &str) -> ConnectionResult {
    let outcome = match status.as_u16() {
        401 | 403 => ConnectionOutcome::AuthenticationFailed,
        _ => ConnectionOutcome::ProtocolMismatch,
    };
    let detail = if status.is_redirection() {
        format!(
            "The service replied with HTTP {} (a redirect). Atlas doesn't follow redirects.",
            status.as_u16()
        )
    } else if outcome == ConnectionOutcome::ProtocolMismatch {
        format!("The service replied with HTTP {}. {hint}", status.as_u16())
    } else {
        format!("The service replied with HTTP {}.", status.as_u16())
    };
    result(outcome, Vec::new(), detail, version)
}

/// Keeps printable, reasonably short identifiers; caps the count.
pub fn sanitize_ids(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    ids.into_iter()
        .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
        .take(MAX_MODELS)
        .collect()
}

fn listed(models: Vec<String>, version: Option<String>) -> ConnectionResult {
    if models.is_empty() {
        result(
            ConnectionOutcome::NoModelsListed,
            models,
            "The service answered with an empty model list.",
            version,
        )
    } else {
        let n = models.len();
        result(
            ConnectionOutcome::Reachable,
            models,
            format!(
                "The service listed {n} model{}.",
                if n == 1 { "" } else { "s" }
            ),
            version,
        )
    }
}

fn failed(fetched: Fetched, version: Option<String>, hint: &str) -> ConnectionResult {
    match fetched {
        Fetched::Status(status, _) => status_result(status, version, hint),
        Fetched::NotJson => result(
            ConnectionOutcome::ProtocolMismatch,
            Vec::new(),
            format!("The answer wasn't JSON. {hint}"),
            version,
        ),
        Fetched::TooLarge => result(
            ConnectionOutcome::ProtocolMismatch,
            Vec::new(),
            "The answer was unexpectedly large.",
            version,
        ),
        Fetched::Failed(outcome, message) => result(outcome, Vec::new(), message, version),
        Fetched::Ok(_) => result(
            ConnectionOutcome::ProtocolMismatch,
            Vec::new(),
            format!("The answer had an unexpected shape. {hint}"),
            version,
        ),
    }
}

/// Ollama's native API: `/api/version` and `/api/tags`.
pub async fn probe_ollama(client: &Client, config: &ProbeConfig, base: &str) -> ConnectionResult {
    const HINT: &str = "Check that this is an Ollama address, usually http://localhost:11434.";
    let version = match fetch_json(client, config, &format!("{base}/api/version"), None).await {
        Fetched::Ok(v) => v.get("version").and_then(Value::as_str).map(str::to_owned),
        Fetched::Failed(outcome, message) => return result(outcome, Vec::new(), message, None),
        _ => None,
    };
    match fetch_json(client, config, &format!("{base}/api/tags"), None).await {
        Fetched::Ok(v) => match v.get("models") {
            Some(Value::Array(models)) => listed(
                sanitize_ids(
                    models
                        .iter()
                        .filter_map(|m| m.get("name")?.as_str().map(str::to_owned)),
                ),
                version,
            ),
            Some(Value::Null) => listed(Vec::new(), version),
            _ => failed(Fetched::Ok(v), version, HINT),
        },
        other => failed(other, version, HINT),
    }
}

/// The OpenAI-compatible model list: `GET {base}/models`.
pub async fn probe_openai(
    client: &Client,
    config: &ProbeConfig,
    base: &str,
    key: Option<&str>,
) -> ConnectionResult {
    const HINT: &str = "Check that the address ends in /v1, like http://localhost:8000/v1.";
    match fetch_json(client, config, &format!("{base}/models"), key).await {
        Fetched::Ok(v) => match v.get("data") {
            Some(Value::Array(models)) => listed(
                sanitize_ids(
                    models
                        .iter()
                        .filter_map(|m| m.get("id")?.as_str().map(str::to_owned)),
                ),
                None,
            ),
            // Ollama's /v1 answers `data: null` when nothing is installed.
            Some(Value::Null) => listed(Vec::new(), None),
            _ => failed(Fetched::Ok(v), None, HINT),
        },
        other => failed(other, None, HINT),
    }
}

// ------------------------------------------------------- capability tests

/// The only text an embedding capability test sends.
pub const EMBED_TEST_INPUT: &str = "Atlas capability test";
/// The only text a chat capability test sends.
pub const CHAT_TEST_PROMPT: &str = "Reply with the word OK.";
/// A reasoning model may spend its first tokens thinking; any streamed
/// token, thought or answer, shows the model chats.
const CHAT_TEST_MAX_TOKENS: u32 = 16;

/// The path and body of an embeddings request for `inputs`.
pub fn embeddings_request(
    kind: ProviderKind,
    model: &str,
    inputs: &[&str],
) -> (&'static str, Value) {
    let path = match kind {
        ProviderKind::Ollama => "/api/embed",
        ProviderKind::OpenaiCompatible => "/embeddings",
    };
    (path, json!({ "model": model, "input": inputs }))
}

/// The path and fixed body of a capability test.
pub fn capability_request(kind: ProviderKind, role: AiRole, model: &str) -> (&'static str, Value) {
    let messages = json!([{ "role": "user", "content": CHAT_TEST_PROMPT }]);
    match (kind, role) {
        (_, AiRole::Embedding) => embeddings_request(kind, model, &[EMBED_TEST_INPUT]),
        (ProviderKind::Ollama, AiRole::Chat) => (
            "/api/chat",
            json!({ "model": model, "messages": messages, "stream": true,
                    "options": { "num_predict": CHAT_TEST_MAX_TOKENS } }),
        ),
        (ProviderKind::OpenaiCompatible, AiRole::Chat) => (
            "/chat/completions",
            json!({ "model": model, "messages": messages, "stream": true,
                    "max_tokens": CHAT_TEST_MAX_TOKENS }),
        ),
    }
}

/// Every embedding in an answer, in input order: Ollama's `embeddings`, or
/// OpenAI's `data`, ordered by each item's `index` when it has one. `None`
/// unless each is a non-empty array of finite numbers.
pub fn embedding_vectors(kind: ProviderKind, answer: &Value) -> Option<Vec<Vec<f32>>> {
    let numbers = |v: &Value| -> Option<Vec<f32>> {
        let values = v.as_array()?;
        if values.is_empty() {
            return None;
        }
        values
            .iter()
            .map(|x| x.as_f64().filter(|f| f.is_finite()).map(|f| f as f32))
            .collect()
    };
    match kind {
        ProviderKind::Ollama => answer
            .get("embeddings")?
            .as_array()?
            .iter()
            .map(numbers)
            .collect(),
        ProviderKind::OpenaiCompatible => {
            let mut items: Vec<(u64, Vec<f32>)> = answer
                .get("data")?
                .as_array()?
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let at = item
                        .get("index")
                        .and_then(Value::as_u64)
                        .unwrap_or(i as u64);
                    Some((at, numbers(item.get("embedding")?)?))
                })
                .collect::<Option<_>>()?;
            items.sort_by_key(|(at, _)| *at);
            Some(items.into_iter().map(|(_, v)| v).collect())
        }
    }
}

/// The longest Retry-After Atlas honours.
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(300);

/// A `Retry-After` given in seconds (an HTTP date is ignored), capped at
/// [`MAX_RETRY_AFTER`].
pub fn retry_after(value: Option<&str>) -> Option<Duration> {
    let secs: u64 = value?.trim().parse().ok()?;
    Some(Duration::from_secs(secs).min(MAX_RETRY_AFTER))
}

/// The answer to an embeddings request, before interpretation.
#[derive(Debug)]
pub enum EmbedReply {
    Ok(Value),
    /// A non-2xx answer with its Retry-After and JSON error message (used
    /// only to classify it).
    Status {
        status: StatusCode,
        retry_after: Option<Duration>,
        message: Option<String>,
    },
    NotJson,
    TooLarge,
    Failed(ConnectionOutcome),
}

/// `POST` an embeddings request and read its JSON answer of at most `cap`
/// bytes, keeping a refusal's Retry-After.
pub async fn post_embeddings(
    client: &Client,
    url: &str,
    bearer: Option<&str>,
    body: &Value,
    cap: usize,
) -> EmbedReply {
    let response = match send(client.post(url).json(body), bearer).await {
        Ok(r) => r,
        Err((outcome, _)) => return EmbedReply::Failed(outcome),
    };
    if !response.status().is_success() {
        let wait = retry_after(
            response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
        );
        return match status_of(response).await {
            Fetched::Status(status, message) => EmbedReply::Status {
                status,
                retry_after: wait,
                message,
            },
            _ => EmbedReply::NotJson,
        };
    }
    match json_of(response, cap).await {
        Fetched::Ok(v) => EmbedReply::Ok(v),
        Fetched::TooLarge => EmbedReply::TooLarge,
        Fetched::Failed(outcome, _) => EmbedReply::Failed(outcome),
        _ => EmbedReply::NotJson,
    }
}

/// The length of the first embedding in an answer, if it is a non-empty
/// array of finite numbers.
pub fn embedding_dimensions(kind: ProviderKind, answer: &Value) -> Option<u32> {
    let vector = match kind {
        ProviderKind::Ollama => answer.get("embeddings")?.get(0)?,
        ProviderKind::OpenaiCompatible => answer.get("data")?.get(0)?.get("embedding")?,
    };
    let values = vector.as_array()?;
    let all_numbers = values
        .iter()
        .all(|v| v.as_f64().is_some_and(f64::is_finite));
    (!values.is_empty() && all_numbers).then(|| u32::try_from(values.len()).ok())?
}

/// What one line of a streamed chat answer says.
#[derive(Debug, PartialEq, Eq)]
pub enum ChatLine {
    /// The model produced a token (an answer or a thought).
    Answered,
    /// The service reported an error inside the stream.
    Error(String),
    /// The stream says it is finished.
    Done,
    Nothing,
}

fn non_empty(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str).is_some_and(|s| !s.is_empty())
}

/// Reads one line of an OpenAI-compatible SSE stream or an Ollama NDJSON
/// stream. A whole non-streamed JSON answer on one line is understood too.
pub fn chat_line(kind: ProviderKind, line: &str) -> ChatLine {
    let line = line.trim();
    let payload = match kind {
        ProviderKind::OpenaiCompatible => match line.strip_prefix("data:") {
            Some(rest) => rest.trim(),
            None if line.starts_with('{') => line,
            None => return ChatLine::Nothing,
        },
        ProviderKind::Ollama => line,
    };
    if payload == "[DONE]" {
        return ChatLine::Done;
    }
    let Ok(v) = serde_json::from_str::<Value>(payload) else {
        return ChatLine::Nothing;
    };
    if v.get("error").is_some() {
        return ChatLine::Error(error_message(&v).unwrap_or_default());
    }
    let answered = match kind {
        ProviderKind::OpenaiCompatible => {
            let choice = v.get("choices").and_then(|c| c.get(0));
            let delta = choice.and_then(|c| c.get("delta"));
            let message = choice.and_then(|c| c.get("message"));
            ["content", "reasoning_content", "reasoning"]
                .iter()
                .any(|k| {
                    non_empty(delta.and_then(|d| d.get(*k)))
                        || non_empty(message.and_then(|m| m.get(*k)))
                })
        }
        ProviderKind::Ollama => {
            let message = v.get("message");
            ["content", "thinking"]
                .iter()
                .any(|k| non_empty(message.and_then(|m| m.get(*k))))
        }
    };
    if answered {
        ChatLine::Answered
    } else if v.get("done").and_then(Value::as_bool) == Some(true) {
        ChatLine::Done
    } else {
        ChatLine::Nothing
    }
}

pub(crate) fn mentions_model_missing(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("model")
        && [
            "not found",
            "no such",
            "does not exist",
            "doesn't exist",
            "unknown",
            "not available",
            "not loaded",
            "try pulling",
        ]
        .iter()
        .any(|p| m.contains(p))
}

fn mentions_wrong_role(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    [
        "embed",
        "not support",
        "unsupported",
        "chat",
        "completion",
        "not a ",
    ]
    .iter()
    .any(|p| m.contains(p))
}

/// Classifies a failed capability request from its status and the error
/// message the service gave (never shown verbatim).
pub fn capability_failure(
    role: AiRole,
    status: Option<StatusCode>,
    message: Option<&str>,
) -> (CapabilityOutcome, String) {
    let code = status.map(|s| s.as_u16());
    let http = code.map_or(String::new(), |c| format!(" (HTTP {c})"));
    let what = match role {
        AiRole::Embedding => "make embeddings",
        AiRole::Chat => "chat",
    };
    if matches!(code, Some(401 | 403)) {
        return (
            CapabilityOutcome::AuthenticationFailed,
            format!("The service refused the key{http}."),
        );
    }
    if status.is_some_and(|s| s.is_redirection()) {
        return (
            CapabilityOutcome::ProtocolMismatch,
            format!("The service replied with a redirect{http}. Atlas doesn't follow redirects."),
        );
    }
    let message = message.unwrap_or_default();
    if mentions_model_missing(message) {
        return (
            CapabilityOutcome::ModelNotFound,
            format!("The service has no model by that name{http}."),
        );
    }
    if !message.is_empty() && mentions_wrong_role(message) {
        return (
            CapabilityOutcome::WrongRole,
            format!("The service says this model can't {what}{http}."),
        );
    }
    let detail = match code {
        Some(404) => format!(
            "The service has no {} endpoint (HTTP 404).",
            match role {
                AiRole::Embedding => "embeddings",
                AiRole::Chat => "chat",
            }
        ),
        Some(c) => format!("The service replied with HTTP {c}."),
        None => "The service answered, but not as expected.".to_owned(),
    };
    (CapabilityOutcome::ProtocolMismatch, detail)
}

/// A capability model id: trimmed, at most 256 characters, printable.
pub fn valid_model_id(model: &str) -> Option<&str> {
    let model = model.trim();
    (!model.is_empty() && model.chars().count() <= 256 && !model.chars().any(char::is_control))
        .then_some(model)
}

fn capability_result(
    role: AiRole,
    model: &str,
    outcome: CapabilityOutcome,
    dimensions: Option<u32>,
    message: impl Into<String>,
    started: Instant,
) -> CapabilityResult {
    CapabilityResult {
        role,
        model_id: model.to_owned(),
        outcome,
        dimensions,
        message: message.into(),
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
        tested_at: now_rfc3339(),
    }
}

fn from_connection(outcome: ConnectionOutcome) -> CapabilityOutcome {
    match outcome {
        ConnectionOutcome::Timeout => CapabilityOutcome::Timeout,
        ConnectionOutcome::AuthenticationFailed => CapabilityOutcome::AuthenticationFailed,
        ConnectionOutcome::InsecureUrlRejected => CapabilityOutcome::InsecureUrlRejected,
        ConnectionOutcome::ProtocolMismatch => CapabilityOutcome::ProtocolMismatch,
        _ => CapabilityOutcome::NetworkError,
    }
}

/// Sends one capability test for `role` to `model` and interprets the
/// answer. An empty model id sends nothing.
pub async fn test_capability(
    client: &Client,
    config: &ProbeConfig,
    kind: ProviderKind,
    base: &str,
    key: Option<&str>,
    role: AiRole,
    model: &str,
) -> CapabilityResult {
    let started = Instant::now();
    let Some(model) = valid_model_id(model) else {
        return capability_result(
            role,
            model.trim(),
            CapabilityOutcome::NeedsModelId,
            None,
            "Choose or type a model first.",
            started,
        );
    };
    let (path, body) = capability_request(kind, role, model);
    let url = format!("{base}{path}");
    let done = |outcome, dimensions, message: String| {
        capability_result(role, model, outcome, dimensions, message, started)
    };
    match role {
        AiRole::Embedding => match post_json(client, &url, key, &body, config.max_body_bytes).await
        {
            Fetched::Ok(answer) => match embedding_dimensions(kind, &answer) {
                Some(n) => done(
                    CapabilityOutcome::Works,
                    Some(n),
                    format!("The model returned an embedding of {n} dimensions."),
                ),
                None => {
                    let (outcome, message) =
                        capability_failure(role, None, error_message(&answer).as_deref());
                    done(outcome, None, message)
                }
            },
            Fetched::Status(status, message) => {
                let (outcome, message) = capability_failure(role, Some(status), message.as_deref());
                done(outcome, None, message)
            }
            Fetched::NotJson => done(
                CapabilityOutcome::ProtocolMismatch,
                None,
                "The answer wasn't JSON.".into(),
            ),
            Fetched::TooLarge => done(
                CapabilityOutcome::ProtocolMismatch,
                None,
                "The answer was unexpectedly large.".into(),
            ),
            Fetched::Failed(outcome, message) => {
                done(from_connection(outcome), None, message.into())
            }
        },
        AiRole::Chat => {
            let (mut answered, mut error) = (false, None::<String>);
            let ended =
                post_stream(
                    client,
                    &url,
                    key,
                    &body,
                    CAPABILITY_STREAM_CAP,
                    |line| match chat_line(kind, line) {
                        ChatLine::Answered => {
                            answered = true;
                            false
                        }
                        ChatLine::Error(message) => {
                            error = Some(message);
                            false
                        }
                        ChatLine::Done => false,
                        ChatLine::Nothing => true,
                    },
                )
                .await;
            match ended {
                _ if answered => done(CapabilityOutcome::Works, None, "The model answered.".into()),
                Streamed::Status(status, message) => {
                    let (outcome, message) =
                        capability_failure(role, Some(status), message.as_deref());
                    done(outcome, None, message)
                }
                Streamed::TooLarge => done(
                    CapabilityOutcome::ProtocolMismatch,
                    None,
                    "The answer was unexpectedly large.".into(),
                ),
                Streamed::Failed(outcome, message) => {
                    done(from_connection(outcome), None, message.into())
                }
                Streamed::Done | Streamed::Stopped => {
                    let (outcome, message) = capability_failure(role, None, error.as_deref());
                    done(outcome, None, message)
                }
            }
        }
    }
}

/// [`ModelProvider`] over HTTP for one provider kind.
pub struct HttpModelProvider {
    pub kind: ProviderKind,
    pub config: ProbeConfig,
}

impl ModelProvider for HttpModelProvider {
    fn kind(&self) -> ProviderKind {
        self.kind
    }

    fn test_connection(
        &self,
        base_url: &str,
        credential: Option<&str>,
    ) -> impl Future<Output = ConnectionResult> + Send {
        let (kind, config) = (self.kind, self.config);
        let base = base_url.trim_end_matches('/').to_owned();
        let key = credential.map(str::to_owned);
        async move {
            let client = client(&config);
            match kind {
                ProviderKind::Ollama => probe_ollama(&client, &config, &base).await,
                ProviderKind::OpenaiCompatible => {
                    probe_openai(&client, &config, &base, key.as_deref()).await
                }
            }
        }
    }

    fn test_capability(
        &self,
        base_url: &str,
        credential: Option<&str>,
        role: AiRole,
        model: &str,
    ) -> impl Future<Output = CapabilityResult> + Send {
        let (kind, config) = (self.kind, self.config);
        let base = base_url.trim_end_matches('/').to_owned();
        let key = credential.map(str::to_owned);
        let model = model.to_owned();
        async move {
            let client = client(&config);
            test_capability(&client, &config, kind, &base, key.as_deref(), role, &model).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_across_chunks_and_endings() {
        let mut buf = b"data: {\"a\"".to_vec();
        assert!(take_lines(&mut buf).is_empty());
        buf.extend_from_slice(b":1}\r\n\ndata: [DONE]\npartial");
        assert_eq!(
            take_lines(&mut buf),
            ["data: {\"a\":1}", "", "data: [DONE]"]
        );
        assert_eq!(buf, b"partial");
    }

    #[test]
    fn chat_lines_from_both_kinds() {
        use ChatLine::*;
        use ProviderKind::*;
        let sse = |s: &str| chat_line(OpenaiCompatible, s);
        assert_eq!(
            sse(r#"data: {"choices":[{"delta":{"content":"OK"}}]}"#),
            Answered
        );
        assert_eq!(
            sse(r#"data: {"choices":[{"delta":{"reasoning_content":"Hmm"}}]}"#),
            Answered,
            "a thinking model's first tokens count"
        );
        assert_eq!(
            sse(r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#),
            Nothing
        );
        assert_eq!(sse("data: [DONE]"), Done);
        assert_eq!(sse(": keep-alive"), Nothing);
        assert_eq!(sse("event: message"), Nothing);
        assert_eq!(sse("data: not json"), Nothing);
        assert_eq!(
            sse(r#"{"choices":[{"message":{"content":"OK"}}]}"#),
            Answered,
            "a non-streamed answer"
        );
        assert_eq!(
            sse(r#"data: {"error":{"message":"model x is not a chat model"}}"#),
            Error("model x is not a chat model".into())
        );
        let nd = |s: &str| chat_line(Ollama, s);
        assert_eq!(
            nd(r#"{"message":{"role":"assistant","content":"OK"},"done":false}"#),
            Answered
        );
        assert_eq!(
            nd(r#"{"message":{"role":"assistant","content":"","thinking":"…"}}"#),
            Answered
        );
        assert_eq!(nd(r#"{"message":{"content":""},"done":true}"#), Done);
        assert_eq!(
            nd(r#"{"error":"model \"x\" not found, try pulling it first"}"#),
            Error("model \"x\" not found, try pulling it first".into())
        );
    }

    #[test]
    fn embeddings_must_be_numbers() {
        let openai = |v: Value| embedding_dimensions(ProviderKind::OpenaiCompatible, &v);
        assert_eq!(
            openai(json!({"data":[{"embedding":[0.1,0.2,0.3]}]})),
            Some(3)
        );
        assert_eq!(openai(json!({"data":[{"embedding":[]}]})), None);
        assert_eq!(
            openai(json!({"data":[{"embedding":"AAAA"}]})),
            None,
            "base64 isn't asked for"
        );
        assert_eq!(openai(json!({"data":[{"embedding":[0.1,"x"]}]})), None);
        assert_eq!(openai(json!({"embeddings":[[0.1]]})), None);
        let ollama = |v: Value| embedding_dimensions(ProviderKind::Ollama, &v);
        assert_eq!(ollama(json!({"embeddings":[vec![0.5; 1024]]})), Some(1024));
        assert_eq!(ollama(json!({"embedding":[0.5]})), None);
    }

    #[test]
    fn failures_are_classified_by_status_and_message() {
        use CapabilityOutcome::*;
        let c = |role, code: u16, m: Option<&str>| {
            capability_failure(role, Some(StatusCode::from_u16(code).unwrap()), m).0
        };
        assert_eq!(c(AiRole::Chat, 401, None), AuthenticationFailed);
        assert_eq!(
            c(AiRole::Embedding, 403, Some("model not found")),
            AuthenticationFailed
        );
        assert_eq!(c(AiRole::Embedding, 302, None), ProtocolMismatch);
        assert_eq!(
            c(
                AiRole::Chat,
                404,
                Some(r#"model "qwen9" not found, try pulling it first"#)
            ),
            ModelNotFound
        );
        assert_eq!(
            c(AiRole::Embedding, 400, Some("The model `x` does not exist")),
            ModelNotFound
        );
        assert_eq!(
            c(AiRole::Embedding, 404, Some("Not Found")),
            ProtocolMismatch,
            "no endpoint"
        );
        assert_eq!(c(AiRole::Embedding, 404, None), ProtocolMismatch);
        assert_eq!(
            c(
                AiRole::Embedding,
                400,
                Some("Qwen3.8 is not an embedding model")
            ),
            WrongRole
        );
        assert_eq!(
            c(AiRole::Chat, 400, Some("this model does not support chat")),
            WrongRole
        );
        assert_eq!(
            c(AiRole::Chat, 500, Some("out of memory")),
            ProtocolMismatch
        );
        assert_eq!(
            capability_failure(AiRole::Chat, None, None).0,
            ProtocolMismatch
        );
    }

    #[test]
    fn error_messages_from_common_shapes() {
        assert_eq!(
            error_message(&json!({"error":{"message":"bad key"}})),
            Some("bad key".into())
        );
        assert_eq!(
            error_message(&json!({"error":"model not found"})),
            Some("model not found".into())
        );
        assert_eq!(
            error_message(&json!({"detail":"Not Found"})),
            Some("Not Found".into())
        );
        assert_eq!(
            error_message(&json!({"message":"nope\u{7}"})),
            Some("nope".into())
        );
        assert_eq!(error_message(&json!({"ok":true})), None);
    }

    #[test]
    fn capability_requests_carry_only_fixed_text() {
        for kind in [ProviderKind::Ollama, ProviderKind::OpenaiCompatible] {
            for role in [AiRole::Chat, AiRole::Embedding] {
                let (_, body) = capability_request(kind, role, "m");
                let text = body.to_string();
                assert!(
                    text.contains(EMBED_TEST_INPUT) || text.contains(CHAT_TEST_PROMPT),
                    "{text}"
                );
            }
        }
        assert_eq!(valid_model_id("  qwen3:8b "), Some("qwen3:8b"));
        assert_eq!(valid_model_id("   "), None);
        assert_eq!(valid_model_id("a\nb"), None);
        assert_eq!(valid_model_id(&"m".repeat(257)), None);
    }

    #[test]
    fn ids_are_sanitized_and_capped() {
        let ids = sanitize_ids(vec![
            "qwen3:8b".into(),
            "".into(),
            "bad\u{7}id".into(),
            "x".repeat(300),
        ]);
        assert_eq!(ids, ["qwen3:8b"]);
        assert_eq!(
            sanitize_ids((0..800).map(|i| format!("m{i}"))).len(),
            MAX_MODELS
        );
    }
}
