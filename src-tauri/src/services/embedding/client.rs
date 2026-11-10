//! The provider-backed embedding backend (Slice 2.3, ADR 0014): the
//! embedding role's service, called from the embedding job's thread.
//!
//! Requests carry the chunk texts to the profile's own address only, with
//! its key, never through a redirect or proxy (ADR 0007). Each request runs
//! on the async runtime while the job's thread waits for it in short steps,
//! so a library closing or the app quitting abandons it at once rather
//! than waiting out its 90-second budget.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use reqwest::StatusCode;

use super::vector::{normalize, truncate_normalize};
use crate::domain::dto::{ConnectionOutcome, ProviderKind};
use crate::domain::traits::{BackendInfo, EmbedError, EmbeddingBackend};
use crate::infrastructure::http_probe::{
    EmbedReply, ProbeConfig, client, embedding_vectors, embeddings_request, mentions_model_missing,
    post_embeddings,
};
use crate::services::providers::secrets::Secret;

/// How often a waiting request checks whether the job is stopping.
const STOP_POLL: Duration = Duration::from_millis(100);

/// Whether the job asked to stop (the runner's signal).
pub type StopCheck = Arc<dyn Fn() -> bool + Send + Sync>;

pub struct ProviderEmbeddingBackend {
    pub kind: ProviderKind,
    /// The profile's normalised base URL (no trailing slash).
    pub base: String,
    pub key: Option<Secret>,
    pub model: String,
    /// The vector length the service returns.
    pub dims: usize,
    /// How many of them Atlas keeps (Matryoshka truncation, Slice 2.4);
    /// equal to `dims` to keep all.
    pub keep: usize,
    pub config: ProbeConfig,
    pub stop: StopCheck,
}

impl ProviderEmbeddingBackend {
    fn request(&self, inputs: &[&str]) -> Result<EmbedReply, EmbedError> {
        let (path, body) = embeddings_request(self.kind, &self.model, inputs);
        let url = format!("{}{path}", self.base);
        let http = client(&self.config);
        let key = self.key.as_ref().map(|k| k.expose().to_owned());
        let cap = self.config.max_body_bytes;
        let (tx, rx) = mpsc::channel();
        let task = tauri::async_runtime::spawn(async move {
            let reply = post_embeddings(&http, &url, key.as_deref(), &body, cap).await;
            let _ = tx.send(reply);
        });
        loop {
            match rx.recv_timeout(STOP_POLL) {
                Ok(reply) => return Ok(reply),
                Err(mpsc::RecvTimeoutError::Timeout) if (self.stop)() => {
                    task.abort();
                    return Err(EmbedError::Cancelled);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(EmbedError::Network),
            }
        }
    }
}

/// Turns an embeddings answer into unit vectors of `dims` numbers, one per
/// input, or the reason it can't be used.
pub fn interpret(
    kind: ProviderKind,
    reply: EmbedReply,
    sent: usize,
    dims: usize,
) -> Result<Vec<Vec<f32>>, EmbedError> {
    match reply {
        EmbedReply::Ok(answer) => {
            let vectors = embedding_vectors(kind, &answer).ok_or(EmbedError::Protocol)?;
            if vectors.len() != sent {
                return Err(EmbedError::WrongCount {
                    sent,
                    got: vectors.len(),
                });
            }
            vectors
                .into_iter()
                .map(|mut v| {
                    if v.len() != dims {
                        return Err(EmbedError::WrongDimensions {
                            expected: dims,
                            got: v.len(),
                        });
                    }
                    normalize(&mut v).then_some(v).ok_or(EmbedError::Protocol)
                })
                .collect()
        }
        EmbedReply::Status {
            status,
            retry_after,
            message,
        } => Err(classify(status, retry_after, message.as_deref())),
        EmbedReply::NotJson | EmbedReply::TooLarge => Err(EmbedError::Protocol),
        EmbedReply::Failed(ConnectionOutcome::Timeout) => Err(EmbedError::Timeout),
        EmbedReply::Failed(_) => Err(EmbedError::Network),
    }
}

/// A refusal's meaning, from its status and error message.
pub fn classify(
    status: StatusCode,
    retry_after: Option<Duration>,
    message: Option<&str>,
) -> EmbedError {
    let code = status.as_u16();
    let model_missing = message.is_some_and(mentions_model_missing);
    match code {
        401 | 403 => EmbedError::Auth,
        429 => EmbedError::RateLimited { retry_after },
        _ if model_missing && matches!(code, 400 | 404 | 422) => EmbedError::ModelNotFound,
        400 | 413 | 422 => EmbedError::BadRequest { status: code },
        500..=599 => EmbedError::Server {
            status: code,
            retry_after,
        },
        // A 404 that names no model (no such endpoint), a redirect, or
        // anything else unexpected.
        _ => EmbedError::Protocol,
    }
}

impl EmbeddingBackend for ProviderEmbeddingBackend {
    fn embed(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let reply = self.request(inputs)?;
        let vectors = interpret(self.kind, reply, inputs.len(), self.dims)?;
        if self.keep >= self.dims {
            return Ok(vectors);
        }
        vectors
            .into_iter()
            .map(|v| truncate_normalize(&v, self.keep).ok_or(EmbedError::Protocol))
            .collect()
    }

    fn describe(&self) -> BackendInfo {
        BackendInfo {
            model: self.model.clone(),
            dims: self.keep.min(self.dims),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ok(v: serde_json::Value) -> EmbedReply {
        EmbedReply::Ok(v)
    }

    #[test]
    fn answers_become_unit_vectors_in_input_order() {
        let openai = ok(json!({ "data": [
            { "index": 1, "embedding": [0.0, 2.0] },
            { "index": 0, "embedding": [3.0, 4.0] },
        ]}));
        let v = interpret(ProviderKind::OpenaiCompatible, openai, 2, 2).unwrap();
        assert_eq!(v, [vec![0.6, 0.8], vec![0.0, 1.0]]);
        let ollama = ok(json!({ "embeddings": [[1.0, 0.0]] }));
        assert_eq!(
            interpret(ProviderKind::Ollama, ollama, 1, 2).unwrap(),
            [vec![1.0, 0.0]]
        );
    }

    #[test]
    fn wrong_answers_are_named() {
        let kind = ProviderKind::OpenaiCompatible;
        let two =
            || ok(json!({ "data": [{ "embedding": [1.0, 0.0] }, { "embedding": [0.0, 1.0] }] }));
        assert_eq!(
            interpret(kind, two(), 3, 2),
            Err(EmbedError::WrongCount { sent: 3, got: 2 })
        );
        assert_eq!(
            interpret(kind, two(), 2, 3),
            Err(EmbedError::WrongDimensions {
                expected: 3,
                got: 2
            })
        );
        let zero = ok(json!({ "data": [{ "embedding": [0.0, 0.0] }] }));
        assert_eq!(interpret(kind, zero, 1, 2), Err(EmbedError::Protocol));
        let text = ok(json!({ "data": [{ "embedding": "AAAA" }] }));
        assert_eq!(interpret(kind, text, 1, 2), Err(EmbedError::Protocol));
        assert_eq!(
            interpret(kind, EmbedReply::NotJson, 1, 2),
            Err(EmbedError::Protocol)
        );
        assert_eq!(
            interpret(kind, EmbedReply::Failed(ConnectionOutcome::Timeout), 1, 2),
            Err(EmbedError::Timeout)
        );
        assert_eq!(
            interpret(
                kind,
                EmbedReply::Failed(ConnectionOutcome::NetworkError),
                1,
                2
            ),
            Err(EmbedError::Network)
        );
    }

    #[test]
    fn refusals_are_classified() {
        let s = |c| StatusCode::from_u16(c).unwrap();
        let wait = Some(Duration::from_secs(7));
        assert_eq!(classify(s(401), None, None), EmbedError::Auth);
        assert_eq!(classify(s(403), None, None), EmbedError::Auth);
        assert_eq!(
            classify(s(429), wait, None),
            EmbedError::RateLimited { retry_after: wait }
        );
        assert_eq!(
            classify(s(503), wait, None),
            EmbedError::Server {
                status: 503,
                retry_after: wait
            }
        );
        assert_eq!(
            classify(s(404), None, Some("model 'x' not found")),
            EmbedError::ModelNotFound
        );
        assert_eq!(
            classify(s(404), None, Some("no route")),
            EmbedError::Protocol
        );
        assert_eq!(
            classify(s(400), None, Some("input too long")),
            EmbedError::BadRequest { status: 400 }
        );
        assert_eq!(classify(s(302), None, None), EmbedError::Protocol);
        assert!(EmbedError::Timeout.is_transient());
        assert!(!EmbedError::Auth.is_transient());
        assert_eq!(classify(s(429), wait, None).retry_after(), wait);
    }
}
