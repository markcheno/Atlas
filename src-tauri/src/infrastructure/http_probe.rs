//! Connection probes for AI services (phase0-spec §10, ADR 0007).
//!
//! A probe only asks the service for its list of models: bodiless `GET`
//! requests, nothing from the library. Redirects are never followed and
//! system proxies are never used, so a probe goes exactly where the user
//! pointed it. Timeouts and the response size are bounded.

use std::future::Future;
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde_json::Value;

use crate::domain::dto::{ConnectionOutcome, ConnectionResult, ProviderKind};
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

/// Result of one GET, before interpretation.
pub enum Fetched {
    Ok(Value),
    Status(StatusCode),
    NotJson,
    TooLarge,
    Failed(ConnectionOutcome, &'static str),
}

pub async fn fetch_json(
    client: &Client,
    config: &ProbeConfig,
    url: &str,
    bearer: Option<&str>,
) -> Fetched {
    let mut request = client.get(url);
    if let Some(key) = bearer {
        request = request.bearer_auth(key);
    }
    let mut response = match request.send().await {
        Ok(r) => r,
        Err(e) if e.is_timeout() => {
            return Fetched::Failed(
                ConnectionOutcome::Timeout,
                "The service took too long to answer.",
            );
        }
        Err(e) if e.is_connect() => {
            return Fetched::Failed(
                ConnectionOutcome::NetworkError,
                "Nothing answered at this address.",
            );
        }
        Err(_) => {
            return Fetched::Failed(
                ConnectionOutcome::NetworkError,
                "The connection failed before the service answered.",
            );
        }
    };
    let status = response.status();
    if !status.is_success() {
        return Fetched::Status(status);
    }
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > config.max_body_bytes {
                    return Fetched::TooLarge;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) if e.is_timeout() => {
                return Fetched::Failed(
                    ConnectionOutcome::Timeout,
                    "The service took too long to answer.",
                );
            }
            Err(_) => {
                return Fetched::Failed(ConnectionOutcome::NetworkError, "The connection dropped.");
            }
        }
    }
    serde_json::from_slice(&body).map_or(Fetched::NotJson, Fetched::Ok)
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
        Fetched::Status(status) => status_result(status, version, hint),
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
