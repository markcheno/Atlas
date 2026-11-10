//! One streamed chat answer (phase2-spec §3, §7.6; ADR 0015).
//!
//! The request goes to the chat role's service only, with its key, never
//! through a redirect or proxy (ADR 0007). It runs on the async runtime
//! while the answer's thread reads its pieces in short steps, so Stop, a
//! new question or the library closing abandons it at once. Two clocks
//! bound it: the first piece (a local runtime may be loading the model)
//! and the silence between pieces.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde_json::Value;

use crate::domain::dto::{AnswerErrorCode, ConnectionOutcome, ProviderKind};
use crate::infrastructure::http_probe::{
    ChatDelta, ProbeConfig, Streamed, chat_delta, chat_request, client, mentions_model_missing,
    post_stream,
};
use crate::services::providers::secrets::Secret;

/// How often a waiting answer checks for Stop.
const STOP_POLL: Duration = Duration::from_millis(50);

/// The chat role's service, resolved.
pub struct ChatTarget {
    pub kind: ProviderKind,
    /// Normalised, without a trailing slash.
    pub base: String,
    pub key: Option<Secret>,
    pub model: String,
}

/// The limits of one answer (phase2-spec §3).
#[derive(Debug, Clone, Copy)]
pub struct ChatLimits {
    pub connect: Duration,
    /// Until the first piece: local runtimes load a model on first use.
    pub first_piece: Duration,
    /// Between pieces once the answer is under way.
    pub idle: Duration,
    /// Bytes of stream read in all (thinking and framing included).
    pub stream_bytes: usize,
    /// Characters of answer text kept.
    pub answer_chars: usize,
}

impl Default for ChatLimits {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(3),
            first_piece: Duration::from_secs(120),
            idle: Duration::from_secs(60),
            stream_bytes: 16 * 1024 * 1024,
            answer_chars: 1024 * 1024,
        }
    }
}

/// A piece of the answer, as it arrives.
#[derive(Debug, PartialEq, Eq)]
pub enum Piece {
    Thinking,
    Text(String),
    /// Nothing new for a moment: a chance to pass on held text.
    Quiet,
}

/// How the stream ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
    Finished,
    Stopped,
    Failed {
        code: AnswerErrorCode,
        message: String,
    },
}

enum Msg {
    Piece(Piece),
    End(Ended),
}

/// Streams the answer to `messages`, handing each piece to `on_piece`,
/// until the service finishes, `stop` says so, or a limit is reached.
/// Blocks the calling thread.
pub fn run(
    target: &ChatTarget,
    limits: ChatLimits,
    messages: &Value,
    stop: &dyn Fn() -> bool,
    mut on_piece: impl FnMut(Piece),
) -> Ended {
    let (path, body) = chat_request(target.kind, &target.model, messages);
    let url = format!("{}{path}", target.base);
    let http = client(&ProbeConfig {
        connect_timeout: limits.connect,
        // The clocks below bound the answer; this is only a backstop.
        total_timeout: Duration::from_secs(30 * 60),
        max_body_bytes: limits.stream_bytes,
    });
    let key = target.key.as_ref().map(|k| k.expose().to_owned());
    let kind = target.kind;
    let (tx, rx) = mpsc::channel::<Msg>();
    let task = tauri::async_runtime::spawn(async move {
        let lines = tx.clone();
        let mut failed = None;
        let streamed = post_stream(
            &http,
            &url,
            key.as_deref(),
            &body,
            limits.stream_bytes,
            |line| match chat_delta(kind, line) {
                ChatDelta::Text(t) => lines.send(Msg::Piece(Piece::Text(t))).is_ok(),
                ChatDelta::Thinking => lines.send(Msg::Piece(Piece::Thinking)).is_ok(),
                ChatDelta::Done => false,
                ChatDelta::Error(message) => {
                    failed = Some(message);
                    false
                }
                ChatDelta::Nothing => true,
            },
        )
        .await;
        let _ = tx.send(Msg::End(match failed {
            Some(message) => service_error(None, Some(&message)),
            None => ended(streamed),
        }));
    });

    let mut filter = ThinkFilter::default();
    let (mut kept, mut started, mut last) = (0usize, false, Instant::now());
    loop {
        if stop() {
            task.abort();
            return Ended::Stopped;
        }
        let wait = if started {
            limits.idle
        } else {
            limits.first_piece
        };
        match rx.recv_timeout(STOP_POLL) {
            Ok(Msg::Piece(piece)) => {
                (started, last) = (true, Instant::now());
                for piece in filter.feed(piece) {
                    if let Piece::Text(t) = &piece {
                        kept += t.chars().count();
                        if kept > limits.answer_chars {
                            task.abort();
                            return failed(AnswerErrorCode::TooLong, "The answer was too long.");
                        }
                    }
                    on_piece(piece);
                }
            }
            Ok(Msg::End(end)) => {
                for piece in filter.finish() {
                    on_piece(piece);
                }
                return end;
            }
            Err(mpsc::RecvTimeoutError::Timeout) if last.elapsed() >= wait => {
                task.abort();
                return failed(
                    AnswerErrorCode::Timeout,
                    "The service stopped answering for too long.",
                );
            }
            Err(mpsc::RecvTimeoutError::Timeout) => on_piece(Piece::Quiet),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return failed(AnswerErrorCode::NetworkError, "The connection dropped.");
            }
        }
    }
}

fn failed(code: AnswerErrorCode, message: &str) -> Ended {
    Ended::Failed {
        code,
        message: message.to_owned(),
    }
}

fn ended(streamed: Streamed) -> Ended {
    match streamed {
        Streamed::Done | Streamed::Stopped => Ended::Finished,
        Streamed::TooLarge => failed(AnswerErrorCode::TooLong, "The answer was too long."),
        Streamed::Status(status, message) => service_error(Some(status), message.as_deref()),
        Streamed::Failed(ConnectionOutcome::Timeout, _) => failed(
            AnswerErrorCode::Timeout,
            "The service took too long to answer.",
        ),
        Streamed::Failed(_, message) => failed(AnswerErrorCode::NetworkError, message),
    }
}

/// A refusal or an error inside the stream, classified. The service's own
/// message only decides the code; it is never shown.
fn service_error(status: Option<StatusCode>, message: Option<&str>) -> Ended {
    let code = status.map(|s| s.as_u16());
    let http = code.map_or(String::new(), |c| format!(" (HTTP {c})"));
    if matches!(code, Some(401 | 403)) {
        return failed(
            AnswerErrorCode::AuthenticationFailed,
            &format!("The service refused the key{http}."),
        );
    }
    if code == Some(404) || message.is_some_and(mentions_model_missing) {
        return failed(
            AnswerErrorCode::ModelNotFound,
            &format!("The service doesn't have the chat model{http}."),
        );
    }
    match code {
        Some(c) if (300..400).contains(&c) => failed(
            AnswerErrorCode::ProtocolMismatch,
            &format!("The service redirected the request{http}; Atlas doesn't follow redirects."),
        ),
        Some(c) if (400..500).contains(&c) && c != 429 => failed(
            AnswerErrorCode::ProtocolMismatch,
            &format!("The service didn't accept the request{http}."),
        ),
        _ => failed(
            AnswerErrorCode::ServiceError,
            &format!("The service reported a problem{http}."),
        ),
    }
}

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

/// Drops a `<think>…</think>` block at the start of the answer text, which
/// some services send inline instead of as a reasoning field.
#[derive(Default)]
pub struct ThinkFilter {
    state: State,
    held: String,
    /// Just after `</think>`: the blank lines before the answer go too.
    trim: bool,
}

#[derive(Default, PartialEq, Eq)]
enum State {
    #[default]
    Start,
    Thinking,
    Answer,
}

impl ThinkFilter {
    pub fn feed(&mut self, piece: Piece) -> Vec<Piece> {
        let Piece::Text(text) = piece else {
            return vec![piece];
        };
        if self.state == State::Answer {
            if !self.trim {
                return vec![Piece::Text(text)];
            }
            let rest = text.trim_start();
            if rest.is_empty() {
                return Vec::new();
            }
            self.trim = false;
            return vec![Piece::Text(rest.to_owned())];
        }
        self.held.push_str(&text);
        let mut out = Vec::new();
        if self.state == State::Start {
            let lead = self.held.trim_start();
            if let Some(rest) = lead.strip_prefix(OPEN) {
                self.held = rest.to_owned();
                self.state = State::Thinking;
            } else if OPEN.starts_with(lead) {
                return out; // Not decided yet.
            } else {
                self.state = State::Answer;
                out.push(Piece::Text(std::mem::take(&mut self.held)));
                return out;
            }
        }
        // Thinking: wait for the closing tag.
        if let Some(at) = self.held.find(CLOSE) {
            let rest = self.held[at + CLOSE.len()..].trim_start().to_owned();
            self.held.clear();
            self.state = State::Answer;
            out.push(Piece::Thinking);
            if rest.is_empty() {
                self.trim = true;
            } else {
                out.push(Piece::Text(rest));
            }
        } else {
            // Keep only what could be the start of the closing tag.
            let keep = self
                .held
                .char_indices()
                .rev()
                .nth(CLOSE.len() - 1)
                .map_or(0, |(i, _)| i);
            self.held.drain(..keep);
            out.push(Piece::Thinking);
        }
        out
    }

    /// The stream ended: undecided text was answer text after all.
    pub fn finish(&mut self) -> Vec<Piece> {
        match self.state {
            State::Start if !self.held.is_empty() => {
                self.state = State::Answer;
                vec![Piece::Text(std::mem::take(&mut self.held))]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_filter(parts: &[&str]) -> (String, usize) {
        let mut f = ThinkFilter::default();
        let (mut text, mut thinking) = (String::new(), 0);
        let mut take = |pieces: Vec<Piece>| {
            for p in pieces {
                match p {
                    Piece::Text(t) => text.push_str(&t),
                    Piece::Thinking => thinking += 1,
                    Piece::Quiet => {}
                }
            }
        };
        for part in parts {
            take(f.feed(Piece::Text((*part).to_owned())));
        }
        take(f.finish());
        (text, thinking)
    }

    #[test]
    fn an_inline_thinking_block_is_dropped_wherever_it_splits() {
        let whole = "<think>Let me see. </th</think>\n\nThe answer [1].";
        for cut in 1..whole.len() {
            if !whole.is_char_boundary(cut) {
                continue;
            }
            let (text, thinking) = run_filter(&[&whole[..cut], &whole[cut..]]);
            assert_eq!(text, "The answer [1].", "cut at {cut}");
            assert!(thinking > 0);
        }
    }

    #[test]
    fn answers_without_thinking_pass_untouched() {
        assert_eq!(run_filter(&["The ", "answer."]).0, "The answer.");
        assert_eq!(run_filter(&["<", "b>bold"]).0, "<b>bold");
        assert_eq!(run_filter(&["<th"]).0, "<th", "a short answer is kept");
        assert_eq!(
            run_filter(&["Fine. <think>x</think>"]).0,
            "Fine. <think>x</think>",
            "only a leading block is thinking"
        );
    }

    #[test]
    fn refusals_are_classified_without_the_services_words() {
        let code = |status: u16, message: Option<&str>| match service_error(
            Some(StatusCode::from_u16(status).unwrap()),
            message,
        ) {
            Ended::Failed { code, message } => {
                assert!(!message.contains("secret detail"));
                code
            }
            other => panic!("{other:?}"),
        };
        assert_eq!(code(401, None), AnswerErrorCode::AuthenticationFailed);
        assert_eq!(code(404, None), AnswerErrorCode::ModelNotFound);
        assert_eq!(
            code(400, Some("model 'x' not found, try pulling it first")),
            AnswerErrorCode::ModelNotFound
        );
        assert_eq!(
            code(400, Some("secret detail")),
            AnswerErrorCode::ProtocolMismatch
        );
        assert_eq!(code(503, None), AnswerErrorCode::ServiceError);
        assert_eq!(code(429, None), AnswerErrorCode::ServiceError);
    }
}
