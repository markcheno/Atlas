//! A tiny scripted HTTP server for AI-service tests (P0-17 onwards): fixed
//! replies by route, every request recorded with its body, and streamed
//! replies for chat. Plus a fake keychain.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_lib::error::AtlasResult;
use atlas_lib::services::providers::secrets::{Secret, SecretStore};

#[derive(Clone)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
    pub delay: Duration,
    /// When set, the body is sent in these pieces with these pauses, with no
    /// `Content-Length` (the connection closes at the end), as a stream.
    pub chunks: Vec<(Vec<u8>, Duration)>,
}

pub fn json(status: u16, body: &str) -> Reply {
    Reply {
        status,
        body: body.as_bytes().to_vec(),
        headers: vec![("Content-Type".into(), "application/json".into())],
        delay: Duration::ZERO,
        chunks: Vec::new(),
    }
}

/// A streamed 200 reply from `pieces`, `pause` apart.
pub fn stream(content_type: &str, pieces: &[&str], pause: Duration) -> Reply {
    Reply {
        status: 200,
        body: Vec::new(),
        headers: vec![("Content-Type".into(), content_type.into())],
        delay: Duration::ZERO,
        chunks: pieces
            .iter()
            .map(|p| (p.as_bytes().to_vec(), pause))
            .collect(),
    }
}

#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub authorization: Option<String>,
    pub content_length: usize,
    pub body: String,
}

pub struct MockServer {
    pub port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl MockServer {
    /// Routes are `"/path"` (any method) or `"POST /path"`.
    pub fn start(routes: Vec<(&str, Reply)>) -> Self {
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

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn serve(mut stream: TcpStream, routes: &HashMap<String, Reply>, log: &Mutex<Vec<Seen>>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut first = head.lines().next().unwrap_or("").split_whitespace();
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
    let content_length: usize = header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < content_length {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    log.lock().unwrap().push(Seen {
        method: method.clone(),
        authorization: header("authorization"),
        content_length,
        body: String::from_utf8_lossy(&body).into_owned(),
        path: path.clone(),
    });
    let reply = routes
        .get(&format!("{method} {path}"))
        .or_else(|| routes.get(&path))
        .cloned()
        .unwrap_or_else(|| json(404, r#"{"error":"not found"}"#));
    std::thread::sleep(reply.delay);
    let mut out = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", reply.status);
    if reply.chunks.is_empty() {
        out.push_str(&format!("Content-Length: {}\r\n", reply.body.len()));
    }
    for (k, v) in &reply.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    let _ = stream.write_all(out.as_bytes());
    if reply.chunks.is_empty() {
        let _ = stream.write_all(&reply.body);
    } else {
        for (piece, pause) in &reply.chunks {
            if stream
                .write_all(piece)
                .and_then(|()| stream.flush())
                .is_err()
            {
                return;
            }
            std::thread::sleep(*pause);
        }
    }
}

pub fn closed_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

#[derive(Default)]
pub struct FakeKeychain {
    pub items: Mutex<HashMap<String, String>>,
    pub broken: bool,
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
