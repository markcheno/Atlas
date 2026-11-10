//! P2-07 and P2-20 for the embedding job's HTTP backend (Slice 2.3): what
//! each answer of a scripted service means, Retry-After, timeouts,
//! cancellation, and the key going only to its own service.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use atlas_lib::domain::dto::ProviderKind;
use atlas_lib::domain::traits::{EmbedError, EmbeddingBackend};
use atlas_lib::infrastructure::http_probe::ProbeConfig;
use atlas_lib::services::embedding::client::ProviderEmbeddingBackend;
use atlas_lib::services::providers::secrets::Secret;

mod common;
use common::http::{MockServer, Reply, closed_port, json};

const DIMS: usize = 4;

fn backend(base: &str, kind: ProviderKind, config: ProbeConfig) -> ProviderEmbeddingBackend {
    ProviderEmbeddingBackend {
        kind,
        base: base.to_owned(),
        key: Some(Secret::new("sk-test-embed-key")),
        model: "test-embed".into(),
        dims: DIMS,
        config,
        stop: Arc::new(|| false),
    }
}

fn vectors(n: usize) -> String {
    let items: Vec<String> = (0..n)
        .map(|i| format!(r#"{{"index": {i}, "embedding": [1.0, {i}.0, 0.0, 0.5]}}"#))
        .collect();
    format!(r#"{{"data": [{}]}}"#, items.join(","))
}

#[test]
fn p2_07_embeddings_are_read_and_each_refusal_is_named() {
    let ok = MockServer::start(vec![("POST /v1/embeddings", json(200, &vectors(2)))]);
    let b = backend(
        &ok.url("/v1"),
        ProviderKind::OpenaiCompatible,
        ProbeConfig::embedding(),
    );
    let got = b.embed(&["one", "two"]).unwrap();
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|v| v.len() == DIMS));
    let norm: f32 = got[0].iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5, "unit length");
    let seen = ok.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/v1/embeddings");
    let body: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(
        body,
        serde_json::json!({ "model": "test-embed", "input": ["one", "two"] })
    );
    // P2-20: the key goes to the profile's own service only.
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some("Bearer sk-test-embed-key")
    );
    assert_eq!(
        b.embed(&["a", "b", "c"]),
        Err(EmbedError::WrongCount { sent: 3, got: 2 })
    );

    let ollama = MockServer::start(vec![(
        "POST /api/embed",
        json(200, r#"{"embeddings": [[0.0, 3.0, 4.0, 0.0]]}"#),
    )]);
    let b = backend(
        &ollama.url(""),
        ProviderKind::Ollama,
        ProbeConfig::embedding(),
    );
    assert_eq!(b.embed(&["x"]).unwrap(), [vec![0.0, 0.6, 0.8, 0.0]]);

    let mut busy = json(429, r#"{"error": {"message": "slow down"}}"#);
    busy.headers.push(("Retry-After".into(), "3".into()));
    let mut down = json(503, "{}");
    down.headers.push(("Retry-After".into(), "9999".into()));
    let refusals = MockServer::start(vec![
        ("POST /busy/embeddings", busy),
        ("POST /down/embeddings", down),
        ("POST /key/embeddings", json(401, r#"{"error": "bad key"}"#)),
        (
            "POST /bad/embeddings",
            json(400, r#"{"error": {"message": "input too long"}}"#),
        ),
        (
            "POST /model/embeddings",
            json(
                404,
                r#"{"error": {"message": "model 'test-embed' not found"}}"#,
            ),
        ),
        (
            "POST /html/embeddings",
            Reply {
                status: 200,
                body: b"<html>hello</html>".to_vec(),
                headers: vec![("Content-Type".into(), "text/html".into())],
                delay: Duration::ZERO,
                chunks: Vec::new(),
            },
        ),
        (
            "POST /dims/embeddings",
            json(200, r#"{"data": [{"embedding": [1.0, 2.0]}]}"#),
        ),
    ]);
    let at = |p: &str| {
        backend(
            &refusals.url(p),
            ProviderKind::OpenaiCompatible,
            ProbeConfig::embedding(),
        )
        .embed(&["x"])
    };
    assert_eq!(
        at("/busy"),
        Err(EmbedError::RateLimited {
            retry_after: Some(Duration::from_secs(3))
        })
    );
    assert_eq!(
        at("/down"),
        Err(EmbedError::Server {
            status: 503,
            retry_after: Some(Duration::from_secs(300))
        }),
        "a long Retry-After is capped"
    );
    assert_eq!(at("/key"), Err(EmbedError::Auth));
    assert_eq!(at("/bad"), Err(EmbedError::BadRequest { status: 400 }));
    assert_eq!(at("/model"), Err(EmbedError::ModelNotFound));
    assert_eq!(at("/html"), Err(EmbedError::Protocol));
    assert_eq!(
        at("/dims"),
        Err(EmbedError::WrongDimensions {
            expected: DIMS,
            got: 2
        })
    );
    assert_eq!(
        at("/nowhere"),
        Err(EmbedError::Protocol),
        "no such endpoint"
    );
}

#[test]
fn p2_07_timeouts_unreachable_services_and_cancelling() {
    let mut slow = json(200, &vectors(1));
    slow.delay = Duration::from_secs(5);
    let server = MockServer::start(vec![("POST /v1/embeddings", slow)]);
    let short = ProbeConfig {
        total_timeout: Duration::from_millis(300),
        ..ProbeConfig::embedding()
    };
    let started = Instant::now();
    let b = backend(&server.url("/v1"), ProviderKind::OpenaiCompatible, short);
    assert_eq!(b.embed(&["x"]), Err(EmbedError::Timeout));
    assert!(started.elapsed() < Duration::from_secs(3));

    let gone = format!("http://127.0.0.1:{}/v1", closed_port());
    let b = backend(
        &gone,
        ProviderKind::OpenaiCompatible,
        ProbeConfig::embedding(),
    );
    assert_eq!(b.embed(&["x"]), Err(EmbedError::Network));

    // A library closing abandons a request at once.
    let stopping = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stopping);
    let mut b = backend(
        &server.url("/v1"),
        ProviderKind::OpenaiCompatible,
        ProbeConfig::embedding(),
    );
    b.stop = Arc::new(move || flag.load(Ordering::Relaxed));
    let started = Instant::now();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        stopping.store(true, Ordering::Relaxed);
    });
    assert_eq!(b.embed(&["x"]), Err(EmbedError::Cancelled));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    stopper.join().unwrap();
}

#[test]
fn p2_20_a_redirect_is_not_followed_and_the_key_stays_home() {
    let elsewhere = MockServer::start(vec![("POST /v1/embeddings", json(200, &vectors(1)))]);
    let mut moved = json(307, "{}");
    moved
        .headers
        .push(("Location".into(), elsewhere.url("/v1/embeddings")));
    let first = MockServer::start(vec![("POST /v1/embeddings", moved)]);
    let b = backend(
        &first.url("/v1"),
        ProviderKind::OpenaiCompatible,
        ProbeConfig::embedding(),
    );
    assert_eq!(b.embed(&["secret text"]), Err(EmbedError::Protocol));
    assert!(
        elsewhere.seen().is_empty(),
        "nothing reached the other host"
    );
    // Errors never carry the key or the text.
    let shown = format!("{:?} {}", EmbedError::Auth, EmbedError::Protocol);
    assert!(!shown.contains("sk-test") && !shown.contains("secret text"));
}
