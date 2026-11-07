//! Slice 0.7: AI service connections (phase0-spec §5.7, §10; P0-17,
//! P0-18, P0-19). Probes run against a tiny scripted HTTP server.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use atlas_lib::domain::dto::{
    ConnectionOutcome, ConnectionResult, ConnectionTestTarget, CredentialStorage, ProviderKind,
    ProviderProfileInput,
};
use atlas_lib::domain::traits::ModelProvider;
use atlas_lib::error::{AtlasResult, ErrorCode};
use atlas_lib::infrastructure::http_probe::{HttpModelProvider, ProbeConfig};
use atlas_lib::services::providers::detect;
use atlas_lib::services::providers::profiles::{self, Prepared};
use atlas_lib::services::providers::secrets::{Secret, SecretStore, SecretVault};
use atlas_lib::services::workspace::Workspace;
use tauri::async_runtime::block_on;

mod common;
use common::{Env, code};

// ---------------------------------------------------------------- mock HTTP

#[derive(Clone)]
struct Reply {
    status: u16,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
    delay: Duration,
}

fn json(status: u16, body: &str) -> Reply {
    Reply {
        status,
        body: body.as_bytes().to_vec(),
        headers: vec![("Content-Type".into(), "application/json".into())],
        delay: Duration::ZERO,
    }
}

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
    content_length: usize,
}

struct MockServer {
    port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl MockServer {
    fn start(routes: Vec<(&str, Reply)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes: HashMap<String, Reply> =
            routes.into_iter().map(|(p, r)| (p.to_owned(), r)).collect();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let routes = routes.clone();
                let log = log.clone();
                std::thread::spawn(move || serve(stream, &routes, &log));
            }
        });
        Self { port, seen }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn serve(mut stream: TcpStream, routes: &HashMap<String, Reply>, log: &Mutex<Vec<Seen>>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("").to_owned();
    let path = first.next().unwrap_or("").to_owned();
    let header = |name: &str| {
        head.lines()
            .find(|l| {
                l.to_ascii_lowercase()
                    .starts_with(&format!("{}:", name.to_ascii_lowercase()))
            })
            .map(|l| l.split_once(':').unwrap().1.trim().to_owned())
    };
    log.lock().unwrap().push(Seen {
        method,
        authorization: header("authorization"),
        content_length: header("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        path: path.clone(),
    });
    let reply = routes
        .get(&path)
        .cloned()
        .unwrap_or_else(|| json(404, r#"{"error":"not found"}"#));
    std::thread::sleep(reply.delay);
    let mut out = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (k, v) in &reply.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.write_all(&reply.body);
}

fn closed_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

// ------------------------------------------------------------ fixtures

#[derive(Default)]
struct FakeKeychain {
    items: Mutex<HashMap<String, String>>,
    broken: bool,
}

impl SecretStore for FakeKeychain {
    fn set(&self, account: &str, secret: &Secret) -> AtlasResult<()> {
        if self.broken {
            return Err(atlas_lib::error::AtlasError::SecretStoreFailed);
        }
        self.items
            .lock()
            .unwrap()
            .insert(account.into(), secret.expose().into());
        Ok(())
    }
    fn get(&self, account: &str) -> AtlasResult<Option<Secret>> {
        Ok(self
            .items
            .lock()
            .unwrap()
            .get(account)
            .map(|s| Secret::new(s.clone())))
    }
    fn delete(&self, account: &str) -> AtlasResult<()> {
        self.items.lock().unwrap().remove(account);
        Ok(())
    }
}

struct Lib {
    env: Env,
    ws: Workspace,
    keychain: Arc<FakeKeychain>,
    vault: SecretVault,
}

impl Lib {
    fn new() -> Self {
        Self::with_keychain(FakeKeychain::default())
    }
    fn with_keychain(keychain: FakeKeychain) -> Self {
        let env = Env::new();
        let mut ws = env.workspace();
        ws.choose(&env.base.join("AtlasData")).unwrap();
        let keychain = Arc::new(keychain);
        let vault = SecretVault::new(Some(keychain.clone() as Arc<dyn SecretStore>));
        Self {
            env,
            ws,
            keychain,
            vault,
        }
    }

    fn save(
        &self,
        input: ProviderProfileInput,
    ) -> AtlasResult<atlas_lib::domain::dto::ProviderProfile> {
        profiles::save(self.ws.require_open().unwrap(), &self.vault, input)
    }

    fn test(&self, target: ConnectionTestTarget) -> ConnectionResult {
        test_with(&self.ws, &self.vault, target, ProbeConfig::default())
    }
}

fn test_with(
    ws: &Workspace,
    vault: &SecretVault,
    target: ConnectionTestTarget,
    config: ProbeConfig,
) -> ConnectionResult {
    match profiles::prepare_test(ws.require_open().unwrap(), vault, &target).unwrap() {
        Prepared::Blocked(result) => result,
        Prepared::Ready {
            kind,
            endpoint,
            key,
        } => block_on(
            HttpModelProvider { kind, config }
                .test_connection(&endpoint.base, key.as_ref().map(|k| k.expose())),
        ),
    }
}

fn input(kind: ProviderKind, base_url: &str) -> ProviderProfileInput {
    ProviderProfileInput {
        id: None,
        kind,
        label: String::new(),
        base_url: base_url.into(),
        api_key: None,
        clear_api_key: false,
        insecure_http_acknowledged: false,
    }
}

fn candidate(kind: ProviderKind, base_url: &str, key: Option<&str>) -> ConnectionTestTarget {
    ConnectionTestTarget::Candidate {
        profile: ProviderProfileInput {
            api_key: key.map(str::to_owned),
            ..input(kind, base_url)
        },
    }
}

// ---------------------------------------------------------------- tests

/// P0-17: an Ollama probe lists models with bodiless GETs only.
#[test]
fn p0_17_ollama_lists_models_without_sending_anything() {
    let server = MockServer::start(vec![
        ("/api/version", json(200, r#"{"version":"0.40.15"}"#)),
        (
            "/api/tags",
            json(
                200,
                r#"{"models":[{"name":"qwen3-embedding:0.6b"},{"name":"qwen3.5:9b"}]}"#,
            ),
        ),
    ]);
    let lib = Lib::new();
    let result = lib.test(candidate(ProviderKind::Ollama, &server.url(""), None));
    assert_eq!(result.outcome, ConnectionOutcome::Reachable);
    assert_eq!(result.model_ids, ["qwen3-embedding:0.6b", "qwen3.5:9b"]);
    assert_eq!(result.server_version.as_deref(), Some("0.40.15"));
    for request in server.seen() {
        assert_eq!(request.method, "GET");
        assert_eq!(request.content_length, 0, "no body is ever sent");
        assert!(request.authorization.is_none());
    }
    assert_eq!(server.seen().len(), 2);
}

#[test]
fn empty_model_lists_are_reported_honestly() {
    let ollama = MockServer::start(vec![
        ("/api/version", json(200, r#"{"version":"0.40.15"}"#)),
        ("/api/tags", json(200, r#"{"models":[]}"#)),
    ]);
    let openai = MockServer::start(vec![(
        "/v1/models",
        json(200, r#"{"object":"list","data":null}"#),
    )]);
    let lib = Lib::new();
    let a = lib.test(candidate(ProviderKind::Ollama, &ollama.url(""), None));
    assert_eq!(a.outcome, ConnectionOutcome::NoModelsListed);
    assert_eq!(a.server_version.as_deref(), Some("0.40.15"));
    let b = lib.test(candidate(
        ProviderKind::OpenaiCompatible,
        &openai.url("/v1"),
        None,
    ));
    assert_eq!(b.outcome, ConnectionOutcome::NoModelsListed);
}

/// P0-18: the key reaches the service and the keychain, nowhere else.
#[test]
fn p0_18_api_key_never_leaves_the_keychain() {
    const KEY: &str = "sk-test-0123456789-SECRET";
    let server = MockServer::start(vec![(
        "/v1/models",
        json(
            200,
            r#"{"data":[{"id":"Qwen3.8-27B-MLX-4bit"},{"id":"bge-m3"}]}"#,
        ),
    )]);
    let lib = Lib::new();
    let saved = lib
        .save(ProviderProfileInput {
            label: "oMLX".into(),
            api_key: Some(KEY.into()),
            ..input(ProviderKind::OpenaiCompatible, &server.url("/v1"))
        })
        .unwrap();
    assert_eq!(saved.credential_storage, CredentialStorage::Keychain);
    assert!(saved.has_credential);
    assert!(saved.on_this_computer);

    let result = lib.test(ConnectionTestTarget::SavedProfile {
        profile_id: saved.id.clone(),
    });
    assert_eq!(result.outcome, ConnectionOutcome::Reachable);
    assert_eq!(result.model_ids.len(), 2);
    assert_eq!(
        server.seen()[0].authorization.as_deref(),
        Some(&*format!("Bearer {KEY}"))
    );

    // Nowhere else: catalog files, IPC JSON, results, Debug output.
    let library = lib.env.base.join("AtlasData/library");
    for file in ["catalog.sqlite", "catalog.sqlite-wal"] {
        if let Ok(bytes) = fs::read(library.join(file)) {
            assert!(
                !bytes.windows(KEY.len()).any(|w| w == KEY.as_bytes()),
                "{file}"
            );
        }
    }
    let listed = profiles::list(lib.ws.require_open().unwrap(), &lib.vault).unwrap();
    for text in [
        serde_json::to_string(&listed).unwrap(),
        serde_json::to_string(&result).unwrap(),
        format!("{result:?} {listed:?}"),
        format!(
            "{:?}",
            ProviderProfileInput {
                api_key: Some(KEY.into()),
                ..input(ProviderKind::OpenaiCompatible, "http://localhost:1")
            }
        ),
    ] {
        assert!(!text.contains(KEY), "{text}");
    }
    assert_eq!(
        lib.keychain
            .items
            .lock()
            .unwrap()
            .get(&format!("provider:{}", saved.id))
            .map(String::as_str),
        Some(KEY)
    );

    profiles::delete(lib.ws.require_open().unwrap(), &lib.vault, &saved.id).unwrap();
    assert!(lib.keychain.items.lock().unwrap().is_empty());
    assert_eq!(
        code(profiles::delete(
            lib.ws.require_open().unwrap(),
            &lib.vault,
            &saved.id
        )),
        ErrorCode::ProfileNotFound
    );
}

#[test]
fn keys_can_be_kept_replaced_and_cleared() {
    let lib = Lib::new();
    let saved = lib
        .save(ProviderProfileInput {
            api_key: Some("first".into()),
            ..input(ProviderKind::OpenaiCompatible, "http://localhost:8000/v1")
        })
        .unwrap();
    let account = format!("provider:{}", saved.id);
    // Editing without a key keeps it.
    lib.save(ProviderProfileInput {
        id: Some(saved.id.clone()),
        label: "Renamed".into(),
        ..input(ProviderKind::OpenaiCompatible, "http://localhost:8000/v1")
    })
    .unwrap();
    assert_eq!(lib.keychain.items.lock().unwrap()[&account], "first");
    lib.save(ProviderProfileInput {
        id: Some(saved.id.clone()),
        api_key: Some("second".into()),
        ..input(ProviderKind::OpenaiCompatible, "http://localhost:8000/v1")
    })
    .unwrap();
    assert_eq!(lib.keychain.items.lock().unwrap()[&account], "second");
    let cleared = lib
        .save(ProviderProfileInput {
            id: Some(saved.id.clone()),
            clear_api_key: true,
            ..input(ProviderKind::OpenaiCompatible, "http://localhost:8000/v1")
        })
        .unwrap();
    assert!(!cleared.has_credential);
    assert!(!lib.keychain.items.lock().unwrap().contains_key(&account));
}

#[test]
fn failures_are_classified() {
    let server = MockServer::start(vec![
        (
            "/auth/models",
            json(401, r#"{"error":{"message":"API key required"}}"#),
        ),
        ("/forbidden/models", json(403, "{}")),
        (
            "/html/models",
            Reply {
                status: 200,
                body: b"<html>hello</html>".to_vec(),
                headers: vec![("Content-Type".into(), "text/html".into())],
                delay: Duration::ZERO,
            },
        ),
        ("/shape/models", json(200, r#"{"models":"nope"}"#)),
        (
            "/huge/models",
            json(
                200,
                &format!(r#"{{"data":[],"pad":"{}"}}"#, "x".repeat(2 * 1024 * 1024)),
            ),
        ),
        (
            "/redirect/models",
            Reply {
                status: 302,
                body: Vec::new(),
                headers: vec![("Location".into(), "http://example.com/models".into())],
                delay: Duration::ZERO,
            },
        ),
        (
            "/slow/models",
            Reply {
                delay: Duration::from_millis(1500),
                ..json(200, r#"{"data":[]}"#)
            },
        ),
    ]);
    let lib = Lib::new();
    let outcome = |path: &str| {
        lib.test(candidate(
            ProviderKind::OpenaiCompatible,
            &server.url(path),
            None,
        ))
        .outcome
    };
    assert_eq!(outcome("/auth"), ConnectionOutcome::AuthenticationFailed);
    assert_eq!(
        outcome("/forbidden"),
        ConnectionOutcome::AuthenticationFailed
    );
    assert_eq!(
        outcome("/nowhere"),
        ConnectionOutcome::ProtocolMismatch,
        "404"
    );
    assert_eq!(outcome("/html"), ConnectionOutcome::ProtocolMismatch);
    assert_eq!(outcome("/shape"), ConnectionOutcome::ProtocolMismatch);
    assert_eq!(outcome("/huge"), ConnectionOutcome::ProtocolMismatch);
    assert_eq!(outcome("/redirect"), ConnectionOutcome::ProtocolMismatch);
    assert!(
        server.seen().iter().all(|s| !s.path.contains("example")),
        "redirect not followed"
    );

    let started = Instant::now();
    let slow = test_with(
        &lib.ws,
        &lib.vault,
        candidate(ProviderKind::OpenaiCompatible, &server.url("/slow"), None),
        ProbeConfig {
            total_timeout: Duration::from_millis(300),
            ..ProbeConfig::default()
        },
    );
    assert_eq!(slow.outcome, ConnectionOutcome::Timeout);
    assert!(started.elapsed() < Duration::from_secs(1));

    let refused = lib.test(candidate(
        ProviderKind::Ollama,
        &format!("http://127.0.0.1:{}", closed_port()),
        None,
    ));
    assert_eq!(refused.outcome, ConnectionOutcome::NetworkError);
}

/// P0-19: plain HTTP leaves this computer only with acknowledgement.
#[test]
fn p0_19_plain_http_rules() {
    let lib = Lib::new();
    let remote = "http://192.168.1.20:11434";
    let blocked = lib.test(candidate(ProviderKind::Ollama, remote, None));
    assert_eq!(blocked.outcome, ConnectionOutcome::InsecureUrlRejected);
    assert_eq!(
        code(lib.save(input(ProviderKind::Ollama, remote))),
        ErrorCode::InsecureHttpNotAcknowledged
    );
    let acknowledged = lib
        .save(ProviderProfileInput {
            insecure_http_acknowledged: true,
            ..input(ProviderKind::Ollama, remote)
        })
        .unwrap();
    assert!(acknowledged.insecure_http_acknowledged);
    assert!(!acknowledged.on_this_computer);
    // The acknowledgement belongs to that profile only.
    assert_eq!(
        code(lib.save(input(ProviderKind::Ollama, "http://192.168.1.21:11434"))),
        ErrorCode::InsecureHttpNotAcknowledged
    );
    assert!(
        lib.save(input(ProviderKind::Ollama, "http://localhost:11434"))
            .is_ok()
    );
    assert!(
        lib.save(input(
            ProviderKind::OpenaiCompatible,
            "https://api.openai.com/v1"
        ))
        .is_ok()
    );
    assert_eq!(
        code(lib.save(input(
            ProviderKind::OpenaiCompatible,
            "https://user:pw@api.example.com/v1"
        ))),
        ErrorCode::UrlHasCredentials
    );
    assert_eq!(
        code(lib.save(input(ProviderKind::Ollama, "localhost:11434"))),
        ErrorCode::InvalidUrl
    );
}

#[test]
fn without_a_keychain_keys_last_for_the_session_only() {
    let lib = Lib::with_keychain(FakeKeychain {
        broken: true,
        ..FakeKeychain::default()
    });
    let saved = lib
        .save(ProviderProfileInput {
            api_key: Some("session-key".into()),
            ..input(ProviderKind::OpenaiCompatible, "http://localhost:8000/v1")
        })
        .unwrap();
    assert_eq!(saved.credential_storage, CredentialStorage::SessionOnly);
    assert!(saved.has_credential);
    let after_restart = SecretVault::new(None);
    let listed = profiles::list(lib.ws.require_open().unwrap(), &after_restart).unwrap();
    assert!(
        !listed[0].has_credential,
        "must be re-entered after restart"
    );
    assert_eq!(listed[0].credential_storage, CredentialStorage::None);
}

#[test]
fn detection_finds_local_services_quickly() {
    let ollama = MockServer::start(vec![(
        "/api/version",
        json(200, r#"{"version":"0.40.15"}"#),
    )]);
    let omlx = MockServer::start(vec![
        ("/v1/models", json(401, r#"{"error":"API key required"}"#)),
        (
            "/health",
            json(200, r#"{"status":"healthy","engine_pool":{}}"#),
        ),
    ]);
    let open = MockServer::start(vec![(
        "/v1/models",
        json(200, r#"{"data":[{"id":"a"},{"id":"b"}]}"#),
    )]);
    let closed = closed_port();
    let ports = [
        (
            ollama.port,
            ProviderKind::Ollama,
            "/api/version",
            "Ollama",
            "",
        ),
        (
            omlx.port,
            ProviderKind::OpenaiCompatible,
            "/v1/models",
            "Local server",
            "/v1",
        ),
        (
            open.port,
            ProviderKind::OpenaiCompatible,
            "/v1/models",
            "LM Studio",
            "/v1",
        ),
        (
            closed,
            ProviderKind::OpenaiCompatible,
            "/v1/models",
            "Nothing",
            "/v1",
        ),
    ];
    let started = Instant::now();
    let found = block_on(detect::detect(&ports, detect::config()));
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(found.len(), 3);
    assert_eq!(found[0].label, "Ollama");
    assert!(!found[0].needs_key);
    assert!(found[1].needs_key, "401 means a key is required");
    assert_eq!(found[2].model_count, Some(2));
    assert!(
        found
            .iter()
            .all(|f| f.base_url.starts_with("http://localhost:"))
    );
}
