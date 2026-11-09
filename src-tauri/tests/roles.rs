//! Slice 2.1: capability tests and model roles (phase2-spec §7.1, §12;
//! P2-01, P2-02, P2-20). Every test runs against the scripted HTTP server;
//! no real model is involved.

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use atlas_lib::domain::dto::{
    AiRole, CapabilityOutcome, CapabilityResult, ConnectionTestTarget, ProviderKind,
    ProviderLocation, ProviderProfile, ProviderProfileInput,
};
use atlas_lib::domain::traits::ModelProvider;
use atlas_lib::error::ErrorCode;
use atlas_lib::infrastructure::http_probe::{
    CHAT_TEST_PROMPT, EMBED_TEST_INPUT, HttpModelProvider, ProbeConfig,
};
use atlas_lib::services::providers::profiles::{self, Prepared};
use atlas_lib::services::providers::roles;
use atlas_lib::services::providers::secrets::{SecretStore, SecretVault};
use atlas_lib::services::workspace::Workspace;
use serde_json::{Value, json};
use tauri::async_runtime::block_on;

mod common;
use common::http::{FakeKeychain, MockServer, Reply, closed_port, json as reply_json, stream};
use common::{Env, code};

const SSE: &str = "text/event-stream";
const NDJSON: &str = "application/x-ndjson";

struct Lib {
    env: Env,
    ws: Workspace,
    vault: SecretVault,
}

impl Lib {
    fn new() -> Self {
        let env = Env::new();
        let mut ws = env.workspace();
        ws.choose(&env.base.join("AtlasData")).unwrap();
        let keychain = Arc::new(FakeKeychain::default());
        let vault = SecretVault::new(Some(keychain as Arc<dyn SecretStore>));
        Self { env, ws, vault }
    }

    fn save(&self, kind: ProviderKind, base_url: &str, key: Option<&str>) -> ProviderProfile {
        profiles::save(
            self.ws.require_open().unwrap(),
            &self.vault,
            ProviderProfileInput {
                id: None,
                kind,
                label: "Local AI".into(),
                base_url: base_url.into(),
                api_key: key.map(str::to_owned),
                clear_api_key: false,
                insecure_http_acknowledged: false,
            },
        )
        .unwrap()
    }

    fn test(&self, profile: &ProviderProfile, role: AiRole, model: &str) -> CapabilityResult {
        self.test_with(profile, role, model, ProbeConfig::capability())
    }

    fn test_with(
        &self,
        profile: &ProviderProfile,
        role: AiRole,
        model: &str,
        config: ProbeConfig,
    ) -> CapabilityResult {
        let target = ConnectionTestTarget::SavedProfile {
            profile_id: profile.id.clone(),
        };
        match profiles::resolve(self.ws.require_open().unwrap(), &self.vault, &target).unwrap() {
            Prepared::Blocked(message) => panic!("blocked: {message}"),
            Prepared::Ready {
                kind,
                endpoint,
                key,
            } => block_on(HttpModelProvider { kind, config }.test_capability(
                &endpoint.base,
                key.as_ref().map(|k| k.expose()),
                role,
                model,
            )),
        }
    }
}

fn vector(n: usize) -> String {
    serde_json::to_string(&vec![0.125_f32; n]).unwrap()
}

/// Every request the server saw carried one of the two fixed test bodies
/// and nothing else.
fn only_fixed_bodies(server: &MockServer) {
    for seen in server.seen() {
        let body: Value = serde_json::from_str(&seen.body).unwrap();
        let text = body.to_string();
        let embed = body.get("input") == Some(&json!([EMBED_TEST_INPUT]));
        let chat =
            body.get("messages") == Some(&json!([{ "role": "user", "content": CHAT_TEST_PROMPT }]));
        assert!(embed ^ chat, "unexpected body {text}");
        assert_eq!(seen.method, "POST");
    }
}

/// P2-01: an embedding test sends one fixed request and reports the
/// model's dimensions, for both kinds of service.
#[test]
fn p2_01_embeddings_work_and_report_dimensions() {
    let server = MockServer::start(vec![
        (
            "POST /v1/embeddings",
            reply_json(
                200,
                &format!(r#"{{"data":[{{"embedding":{}}}]}}"#, vector(1024)),
            ),
        ),
        (
            "POST /api/embed",
            reply_json(200, &format!(r#"{{"embeddings":[{}]}}"#, vector(768))),
        ),
    ]);
    let lib = Lib::new();
    let omlx = lib.save(ProviderKind::OpenaiCompatible, &server.url("/v1"), None);
    let r = lib.test(
        &omlx,
        AiRole::Embedding,
        "mlx-community/Qwen3-Embedding-0.6B-8bit",
    );
    assert_eq!(r.outcome, CapabilityOutcome::Works, "{r:?}");
    assert_eq!(r.dimensions, Some(1024));
    assert_eq!(r.model_id, "mlx-community/Qwen3-Embedding-0.6B-8bit");

    let ollama = lib.save(ProviderKind::Ollama, &server.url(""), None);
    let r = lib.test(&ollama, AiRole::Embedding, "qwen3-embedding:0.6b");
    assert_eq!(
        (r.outcome, r.dimensions),
        (CapabilityOutcome::Works, Some(768))
    );

    let seen = server.seen();
    assert_eq!(
        seen.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(),
        ["/v1/embeddings", "/api/embed"]
    );
    let first: Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(
        first,
        json!({ "model": "mlx-community/Qwen3-Embedding-0.6B-8bit", "input": [EMBED_TEST_INPUT] })
    );
    only_fixed_bodies(&server);
}

/// P2-01: a chat test reads the streamed answer and passes on the first
/// token, a thought included; it never returns the model's words.
#[test]
fn p2_01_chat_works_on_streamed_tokens_and_thoughts() {
    let server = MockServer::start(vec![
        (
            "POST /v1/chat/completions",
            stream(
                SSE,
                &[
                    ": keep-alive\n\n",
                    "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{\"reasoning_",
                    "content\":\"The user wants OK\"}}]}\n\n",
                    "data: [DONE]\n\n",
                ],
                Duration::from_millis(20),
            ),
        ),
        (
            "POST /api/chat",
            stream(
                NDJSON,
                &[
                    "{\"message\":{\"role\":\"assistant\",\"content\":\"OK\"},\"done\":false}\n",
                    "{\"message\":{\"content\":\"\"},\"done\":true}\n",
                ],
                Duration::ZERO,
            ),
        ),
    ]);
    let lib = Lib::new();
    let omlx = lib.save(ProviderKind::OpenaiCompatible, &server.url("/v1"), None);
    let r = lib.test(&omlx, AiRole::Chat, "Qwen3.8-27B-MLX-4bit");
    assert_eq!(r.outcome, CapabilityOutcome::Works, "{r:?}");
    assert_eq!(r.dimensions, None);
    assert!(!r.message.contains("The user wants"), "the reply stays out");

    let ollama = lib.save(ProviderKind::Ollama, &server.url(""), None);
    let r = lib.test(&ollama, AiRole::Chat, "qwen3.5:9b");
    assert_eq!(r.outcome, CapabilityOutcome::Works, "{r:?}");

    let body: Value = serde_json::from_str(&server.seen()[0].body).unwrap();
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["max_tokens"], json!(16));
    only_fixed_bodies(&server);
}

/// P2-01: every failure is reported for what it is, and an empty model id
/// sends nothing.
#[test]
fn p2_01_failures_are_honest() {
    let server = MockServer::start(vec![
        (
            "POST /missing/api/embed",
            reply_json(
                404,
                r#"{"error":"model \"nope\" not found, try pulling it first"}"#,
            ),
        ),
        (
            "POST /wrong/v1/embeddings",
            reply_json(
                400,
                r#"{"error":{"message":"Qwen3.8-27B-MLX-4bit is not an embedding model"}}"#,
            ),
        ),
        (
            "POST /wrongchat/v1/chat/completions",
            stream(
                SSE,
                &["data: {\"error\":{\"message\":\"bge-m3 does not support chat\"}}\n\n"],
                Duration::ZERO,
            ),
        ),
        (
            "POST /noroute/v1/embeddings",
            reply_json(404, r#"{"detail":"Not Found"}"#),
        ),
        (
            "POST /auth/v1/embeddings",
            reply_json(401, r#"{"error":{"message":"invalid api key"}}"#),
        ),
        (
            "POST /html/v1/embeddings",
            Reply {
                body: b"<html>hello</html>".to_vec(),
                headers: vec![("Content-Type".into(), "text/html".into())],
                ..reply_json(200, "")
            },
        ),
        (
            "POST /shape/v1/embeddings",
            reply_json(200, r#"{"data":[{"embedding":[]}]}"#),
        ),
        (
            "POST /silent/v1/chat/completions",
            stream(SSE, &["data: [DONE]\n\n"], Duration::ZERO),
        ),
        (
            "POST /moved/v1/embeddings",
            Reply {
                headers: vec![("Location".into(), "http://example.com/v1/embeddings".into())],
                ..reply_json(302, "")
            },
        ),
        (
            "POST /slow/v1/embeddings",
            Reply {
                delay: Duration::from_millis(1500),
                ..reply_json(
                    200,
                    &format!(r#"{{"data":[{{"embedding":{}}}]}}"#, vector(8)),
                )
            },
        ),
    ]);
    let lib = Lib::new();
    let openai = |path: &str| lib.save(ProviderKind::OpenaiCompatible, &server.url(path), None);
    use AiRole::{Chat, Embedding};
    use CapabilityOutcome::*;

    let empty = lib.test(&openai("/v1"), Embedding, "   ");
    assert_eq!(empty.outcome, NeedsModelId);
    assert!(server.seen().is_empty(), "nothing is sent without a model");

    let ollama = lib.save(ProviderKind::Ollama, &server.url("/missing"), None);
    let cases = [
        (lib.test(&ollama, Embedding, "nope"), ModelNotFound),
        (
            lib.test(&openai("/wrong/v1"), Embedding, "Qwen3.8-27B-MLX-4bit"),
            WrongRole,
        ),
        (
            lib.test(&openai("/wrongchat/v1"), Chat, "bge-m3"),
            WrongRole,
        ),
        (
            lib.test(&openai("/noroute/v1"), Embedding, "m"),
            ProtocolMismatch,
        ),
        (
            lib.test(&openai("/auth/v1"), Embedding, "m"),
            AuthenticationFailed,
        ),
        (
            lib.test(&openai("/html/v1"), Embedding, "m"),
            ProtocolMismatch,
        ),
        (
            lib.test(&openai("/shape/v1"), Embedding, "m"),
            ProtocolMismatch,
        ),
        (lib.test(&openai("/silent/v1"), Chat, "m"), ProtocolMismatch),
        (
            lib.test(&openai("/moved/v1"), Embedding, "m"),
            ProtocolMismatch,
        ),
    ];
    for (result, want) in cases {
        assert_eq!(result.outcome, want, "{result:?}");
        assert_eq!(result.dimensions, None);
        assert!(
            !result.message.contains("invalid api key"),
            "server text stays out"
        );
    }

    let short = ProbeConfig {
        total_timeout: Duration::from_millis(400),
        ..ProbeConfig::capability()
    };
    let slow = lib.test_with(&openai("/slow/v1"), Embedding, "m", short);
    assert_eq!(slow.outcome, Timeout);

    let gone = lib.save(
        ProviderKind::OpenaiCompatible,
        &format!("http://127.0.0.1:{}/v1", closed_port()),
        None,
    );
    assert_eq!(lib.test(&gone, Chat, "m").outcome, NetworkError);
    only_fixed_bodies(&server);
}

fn works(lib: &Lib, profile: &ProviderProfile, role: AiRole, model: &str) -> CapabilityResult {
    let r = lib.test(profile, role, model);
    assert_eq!(r.outcome, CapabilityOutcome::Works, "{r:?}");
    r
}

/// P2-02: roles need a matching passing test, persist, and follow their
/// service.
#[test]
fn p2_02_roles_need_a_passing_test_and_follow_their_service() {
    let server = MockServer::start(vec![
        (
            "POST /v1/embeddings",
            reply_json(
                200,
                &format!(r#"{{"data":[{{"embedding":{}}}]}}"#, vector(1024)),
            ),
        ),
        (
            "POST /v1/chat/completions",
            stream(
                SSE,
                &["data: {\"choices\":[{\"delta\":{\"content\":\"OK\"}}]}\n\n"],
                Duration::ZERO,
            ),
        ),
    ]);
    let mut lib = Lib::new();
    let profile = lib.save(ProviderKind::OpenaiCompatible, &server.url("/v1"), None);
    let root = || lib.ws.require_open().unwrap();
    assert_eq!(roles::get(root()).unwrap(), Default::default());

    let embed = works(&lib, &profile, AiRole::Embedding, "qwen3-embedding:0.6b");
    let chat = works(&lib, &profile, AiRole::Chat, "qwen3.5:9b");

    // A pass for another role or model doesn't count.
    let wrong = roles::set(root(), AiRole::Chat, &profile.id, "qwen3.5:9b", &embed);
    assert_eq!(code(wrong), ErrorCode::ModelNotTested);
    let other = roles::set(root(), AiRole::Embedding, &profile.id, "bge-m3", &embed);
    assert_eq!(code(other), ErrorCode::ModelNotTested);
    let mut failed = embed.clone();
    failed.outcome = CapabilityOutcome::WrongRole;
    let failed = roles::set(
        root(),
        AiRole::Embedding,
        &profile.id,
        &embed.model_id,
        &failed,
    );
    assert_eq!(code(failed), ErrorCode::ModelNotTested);
    let mut stale = embed.clone();
    stale.tested_at = "2026-01-01T00:00:00Z".into();
    let stale = roles::set(
        root(),
        AiRole::Embedding,
        &profile.id,
        &embed.model_id,
        &stale,
    );
    assert_eq!(code(stale), ErrorCode::ModelNotTested);
    let missing = roles::set(
        root(),
        AiRole::Embedding,
        "no-such-id",
        &embed.model_id,
        &embed,
    );
    assert_eq!(code(missing), ErrorCode::ProfileNotFound);

    roles::set(
        root(),
        AiRole::Embedding,
        &profile.id,
        &embed.model_id,
        &embed,
    )
    .unwrap();
    let set = roles::set(root(), AiRole::Chat, &profile.id, &chat.model_id, &chat).unwrap();
    let e = set.embedding.as_ref().unwrap();
    assert_eq!(
        (
            e.model_id.as_str(),
            e.dimensions,
            e.profile_label.as_str(),
            e.location
        ),
        (
            "qwen3-embedding:0.6b",
            Some(1024),
            "Local AI",
            ProviderLocation::ThisComputer
        )
    );
    assert_eq!(set.chat.as_ref().unwrap().model_id, "qwen3.5:9b");

    // Kept across a restart.
    lib.ws = lib.env.workspace();
    lib.ws.startup();
    let root = || lib.ws.require_open().unwrap();
    assert_eq!(roles::get(root()).unwrap(), set);

    let cleared = roles::clear(root(), AiRole::Chat).unwrap();
    assert!(cleared.chat.is_none() && cleared.embedding.is_some());

    // Removing the service clears the roles it held.
    profiles::delete(root(), &lib.vault, &profile.id).unwrap();
    assert_eq!(roles::get(root()).unwrap(), Default::default());
}

/// P2-20: the key goes only to the service's own address, never through a
/// redirect, and never into results, roles, events or the catalog.
#[test]
fn p2_20_keys_go_only_to_their_service() {
    const KEY: &str = "sk-capability-0123456789-SECRET";
    let elsewhere = MockServer::start(vec![(
        "POST /v1/embeddings",
        reply_json(
            200,
            &format!(r#"{{"data":[{{"embedding":{}}}]}}"#, vector(4)),
        ),
    )]);
    let server = MockServer::start(vec![
        (
            "POST /v1/embeddings",
            reply_json(
                200,
                &format!(r#"{{"data":[{{"embedding":{}}}]}}"#, vector(1024)),
            ),
        ),
        (
            "POST /moved/v1/embeddings",
            Reply {
                headers: vec![("Location".into(), elsewhere.url("/v1/embeddings"))],
                ..reply_json(307, "")
            },
        ),
    ]);
    let lib = Lib::new();
    let profile = lib.save(
        ProviderKind::OpenaiCompatible,
        &server.url("/v1"),
        Some(KEY),
    );
    let r = works(&lib, &profile, AiRole::Embedding, "qwen3-embedding:0.6b");
    assert_eq!(
        server.seen()[0].authorization.as_deref(),
        Some(&*format!("Bearer {KEY}"))
    );

    let moved = lib.save(
        ProviderKind::OpenaiCompatible,
        &server.url("/moved/v1"),
        Some(KEY),
    );
    let redirected = lib.test(&moved, AiRole::Embedding, "m");
    assert_eq!(redirected.outcome, CapabilityOutcome::ProtocolMismatch);
    assert!(elsewhere.seen().is_empty(), "the redirect is not followed");

    let root = lib.ws.require_open().unwrap();
    let assigned = roles::set(root, AiRole::Embedding, &profile.id, &r.model_id, &r).unwrap();
    for text in [
        serde_json::to_string(&r).unwrap(),
        serde_json::to_string(&redirected).unwrap(),
        serde_json::to_string(&assigned).unwrap(),
        format!("{r:?} {redirected:?} {assigned:?}"),
    ] {
        assert!(!text.contains(KEY));
    }
    let library = lib.env.base.join("AtlasData/library");
    for file in ["catalog.sqlite", "catalog.sqlite-wal"] {
        if let Ok(bytes) = fs::read(library.join(file)) {
            assert!(
                !bytes.windows(KEY.len()).any(|w| w == KEY.as_bytes()),
                "{file}"
            );
        }
    }
}
