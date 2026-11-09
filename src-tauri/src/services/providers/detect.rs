//! Finds AI services answering on this computer. Runs only when the user
//! asks (no startup probes) and only contacts loopback ports.

use std::time::Duration;

use reqwest::StatusCode;
use serde_json::Value;

use crate::domain::dto::{LocalProviderSuggestion, ProviderKind};
use crate::infrastructure::http_probe::{self, Fetched, ProbeConfig};

/// Well-known local servers: (port, kind, probe path, label, base path).
pub const KNOWN_PORTS: &[(u16, ProviderKind, &str, &str, &str)] = &[
    (11434, ProviderKind::Ollama, "/api/version", "Ollama", ""),
    (
        1234,
        ProviderKind::OpenaiCompatible,
        "/v1/models",
        "LM Studio",
        "/v1",
    ),
    (
        8000,
        ProviderKind::OpenaiCompatible,
        "/v1/models",
        "Local server on port 8000",
        "/v1",
    ),
    (
        8080,
        ProviderKind::OpenaiCompatible,
        "/v1/models",
        "Local server on port 8080",
        "/v1",
    ),
];

pub fn config() -> ProbeConfig {
    ProbeConfig {
        connect_timeout: Duration::from_millis(500),
        total_timeout: Duration::from_secs(1),
        max_body_bytes: 1024 * 1024,
    }
}

/// Probes `ports` on 127.0.0.1 in parallel.
pub async fn detect(
    ports: &[(u16, ProviderKind, &str, &str, &str)],
    config: ProbeConfig,
) -> Vec<LocalProviderSuggestion> {
    let client = http_probe::client(&config);
    let probes: Vec<_> = ports
        .iter()
        .map(|&(port, kind, path, label, base_path)| {
            let client = client.clone();
            let (path, label, base_path) =
                (path.to_owned(), label.to_owned(), base_path.to_owned());
            tauri::async_runtime::spawn(async move {
                let origin = format!("http://127.0.0.1:{port}");
                let (needs_key, model_count) = match http_probe::fetch_json(
                    &client,
                    &config,
                    &format!("{origin}{path}"),
                    None,
                )
                .await
                {
                    Fetched::Ok(v) => (false, count_models(kind, &v)),
                    Fetched::Status(s, _)
                        if s == StatusCode::UNAUTHORIZED || s == StatusCode::FORBIDDEN =>
                    {
                        (true, None)
                    }
                    _ => return None,
                };
                let mut label = label;
                // oMLX identifies itself on its unauthenticated health endpoint.
                if port == 8000
                    && let Fetched::Ok(health) =
                        http_probe::fetch_json(&client, &config, &format!("{origin}/health"), None)
                            .await
                    && health.get("engine_pool").is_some()
                {
                    label = "oMLX".to_owned();
                }
                Some(LocalProviderSuggestion {
                    kind,
                    label,
                    base_url: format!("http://localhost:{port}{base_path}"),
                    needs_key,
                    model_count,
                })
            })
        })
        .collect();
    let mut found = Vec::new();
    // All probes are already running concurrently; this just collects them.
    for probe in probes {
        if let Ok(Some(suggestion)) = probe.await {
            found.push(suggestion);
        }
    }
    found
}

fn count_models(kind: ProviderKind, v: &Value) -> Option<u32> {
    match kind {
        // `/api/version` doesn't list models.
        ProviderKind::Ollama => None,
        ProviderKind::OpenaiCompatible => match v.get("data") {
            Some(Value::Array(a)) => u32::try_from(a.len()).ok(),
            Some(Value::Null) => Some(0),
            _ => None,
        },
    }
}
