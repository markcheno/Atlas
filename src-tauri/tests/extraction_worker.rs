//! Slice 1.2: the extraction worker skeleton (phase1-spec §7.2). Spawns the
//! real executable as `atlas --extract-worker` and speaks to it over pipes.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use atlas_lib::services::extraction::locate::{LIBRARY_ENV, Unavailable};
use atlas_lib::services::extraction::protocol::Frame;
use atlas_lib::services::extraction::worker_main::{
    EXIT_OK, EXIT_UNAVAILABLE, TEST_STDOUT_NOISE_ENV,
};
use atlas_lib::services::extraction::{PDFIUM_VERSION, PROTOCOL_VERSION, WORKER_ARG};

/// Bounds a failure, not a performance claim.
const LONG: Duration = Duration::from_secs(30);

struct Worker {
    child: Child,
    lines: mpsc::Receiver<String>,
}

impl Worker {
    fn spawn(env: &[(&str, &str)]) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_atlas"));
        cmd.arg(WORKER_ARG)
            .env_remove(LIBRARY_ENV)
            .env_remove(TEST_STDOUT_NOISE_ENV)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("start the worker");
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self { child, lines }
    }

    fn frame(&self) -> Frame {
        let line = self
            .lines
            .recv_timeout(LONG)
            .expect("a line from the worker");
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("not a frame: {line:?} ({e})"))
    }

    fn send(&mut self, line: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    /// Closes stdin and returns the exit code, the remaining stdout lines
    /// and stderr.
    fn finish(mut self) -> (i32, Vec<String>, String) {
        drop(self.child.stdin.take());
        let start = Instant::now();
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(start.elapsed() < LONG, "the worker did not exit");
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut stderr = String::new();
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        let rest = self.lines.try_iter().collect();
        (status.code().unwrap_or(-1), rest, stderr)
    }
}

fn assert_hello(frame: &Frame) {
    match frame {
        Frame::Hello {
            protocol,
            extractor_version,
            pdfium_version,
            self_test,
        } => {
            assert_eq!(*protocol, PROTOCOL_VERSION);
            assert_eq!(extractor_version, env!("CARGO_PKG_VERSION"));
            assert_eq!(pdfium_version, PDFIUM_VERSION);
            assert_eq!(self_test.pages, 2);
            assert!(self_test.chars > 1000, "{self_test:?}");
        }
        other => panic!("expected hello, got {other:?}"),
    }
}

#[test]
fn the_worker_loads_pdfium_and_says_hello() {
    let mut w = Worker::spawn(&[]);
    assert_hello(&w.frame());

    w.send(r#"{"type":"extract","path":"x"}"#);
    assert_eq!(
        w.frame(),
        Frame::Error {
            reason: "unknown_request".into()
        }
    );
    w.send("not json");
    assert_eq!(
        w.frame(),
        Frame::Error {
            reason: "invalid_request".into()
        }
    );
    let (code, rest, stderr) = w.finish();
    assert_eq!(code, EXIT_OK, "stderr: {stderr}");
    assert!(rest.is_empty(), "{rest:?}");
}

#[test]
fn a_missing_library_is_reported_not_a_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("no-such-pdfium");
    let w = Worker::spawn(&[(LIBRARY_ENV, missing.to_str().unwrap())]);
    assert_eq!(
        w.frame(),
        Frame::Unavailable {
            reason: Unavailable::NotFound
        }
    );
    let (code, rest, stderr) = w.finish();
    assert_eq!(code, EXIT_UNAVAILABLE);
    assert!(rest.is_empty());
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn a_file_that_is_not_pdfium_is_reported_not_a_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = tmp.path().join("fake-pdfium");
    std::fs::write(&fake, b"not a library").unwrap();
    let w = Worker::spawn(&[(LIBRARY_ENV, fake.to_str().unwrap())]);
    assert_eq!(
        w.frame(),
        Frame::Unavailable {
            reason: Unavailable::LoadFailed
        }
    );
    let (code, _, stderr) = w.finish();
    assert_eq!(code, EXIT_UNAVAILABLE);
    assert!(!stderr.contains("panicked"), "{stderr}");
}

/// Anything written to file descriptor 1 (as a library might) goes to
/// stderr; the protocol stream stays pure JSON lines.
#[test]
fn stray_stdout_output_never_reaches_the_protocol() {
    let w = Worker::spawn(&[(TEST_STDOUT_NOISE_ENV, "1")]);
    assert_hello(&w.frame());
    let (code, rest, stderr) = w.finish();
    assert_eq!(code, EXIT_OK);
    assert!(rest.is_empty(), "{rest:?}");
    assert!(stderr.contains("noise on fd 1"), "{stderr:?}");
    assert!(stderr.contains("noise from println"), "{stderr:?}");
}

/// Reported, not asserted: the time from spawn to `hello` decides whether
/// the worker can stay a mode of the app executable (phase1-spec §7.2).
#[test]
fn time_to_hello() {
    let mut times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let w = Worker::spawn(&[]);
        assert_hello(&w.frame());
        times.push(start.elapsed());
        w.finish();
    }
    times.sort();
    eprintln!(
        "worker time to hello (debug build): median {:?}, max {:?}",
        times[times.len() / 2],
        times[times.len() - 1]
    );
}
