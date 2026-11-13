//! Slice 4.1, phase4-spec §7.1 and §12: the log (P4-01), the report
//! (P4-03) and a worker of another protocol refused by the job (P4-04).
//! P4-02, no words in the log or the report, extends P3-16 in
//! `recording_privacy.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use atlas_lib::domain::dto::{
    JobKind, JobState, KnowledgeState, PauseReason, PolicyMode, PowerSource,
};
use atlas_lib::services::diagnostics::event::{FailureReason, PauseBy, WorkerRole};
use atlas_lib::services::diagnostics::report::{self, Inputs, KnowledgeSummary, Report};
use atlas_lib::services::catalog::migrations::SUPPORTED_SCHEMA_VERSION;
use atlas_lib::services::diagnostics::{self, Diagnostics, Event, LOGS_DIR, LogLine};
use atlas_lib::services::extraction::db;
use atlas_lib::services::extraction::supervisor::{Crash, WorkerStats};
use atlas_lib::services::extraction::worker_main::{TEST_FAULT_ENV, TEST_PROTOCOL_ENV};
use atlas_lib::services::workspace::Workspace;
use atlas_lib::services::{inventory, sources, storage};
use rusqlite::Connection;

mod common;
use common::{Env, ExtractionHarness, fast_extraction, wait_until, worker};

const LONG: Duration = Duration::from_secs(180);

fn corpus(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spikes/extraction-bench/corpus")
        .join(rel)
}

fn fixture(dir: &Path, files: u32) {
    let out = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
        .arg(dir)
        .args(["--files", &files.to_string(), "--text-kb", "4"])
        .output()
        .expect("node runs the fixture generator");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn settled(ws: &Workspace) -> bool {
    let root = ws.require_open().unwrap();
    let inv = inventory::control::overview(root).unwrap();
    let ov = root
        .reader()
        .read(|c| db::overview(c, None))
        .unwrap()
        .unwrap();
    inv.active.is_none()
        && inv.queued_sessions == 0
        && ov.active.is_none()
        && ov.queued_jobs == 0
        && ov.counts.pending == 0
}

fn data(env: &Env) -> PathBuf {
    env.base.join("AtlasData")
}

fn events(env: &Env) -> Vec<Event> {
    diagnostics::tail(&data(env).join(LOGS_DIR), 100_000)
        .into_iter()
        .map(|l| l.event)
        .collect()
}

fn log_text(env: &Env) -> String {
    fs::read_dir(data(env).join(LOGS_DIR))
        .unwrap()
        .flatten()
        .map(|e| fs::read_to_string(e.path()).unwrap())
        .collect()
}

/// Where `want` is in `events`, after `from`.
fn find(events: &[Event], from: usize, want: impl Fn(&Event) -> bool) -> Option<usize> {
    events[from..].iter().position(want).map(|i| i + from)
}

/// A library of `files` generated books and `extra` corpus files (copied
/// under the names given), read once.
fn read_library(env: &Env, h: &ExtractionHarness, files: u32, extra: &[(&str, &str)]) -> Workspace {
    let books = env.base.join("Books");
    fixture(&books, files);
    for (rel, name) in extra {
        fs::copy(corpus(rel), books.join(name)).unwrap();
    }
    let mut ws = env.workspace_with_extraction(h);
    h.inv.deps.governor.lock().unwrap().diagnostics = Some(ws.diagnostics().clone());
    ws.choose(&data(env)).unwrap();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &books).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    assert!(wait_until(LONG, || settled(&ws)), "never settled");
    ws
}

/// P4-01: a session of cataloging, reading (with a worker that dies on one
/// file), a battery pause and closing is logged as typed events, in order,
/// each pause once, and nothing names a file.
#[test]
fn p4_01_a_session_is_logged_as_codes_and_numbers() {
    let env = Env::new();
    let h = ExtractionHarness::new(
        fast_extraction(),
        worker().env(TEST_FAULT_ENV, "crash-exit:exit"),
    );
    let ws = read_library(&env, &h, 4, &[("pdf/generated/book.pdf", "crash-exit.pdf")]);

    // On a low battery a rescan pauses, re-checked many times, then
    // resumes when plugged in.
    h.inv.monitor.set(|s| {
        s.power = PowerSource::Battery;
        s.battery_present = Some(true);
        s.battery_percent = Some(10.0);
    });
    let root = ws.require_open().unwrap();
    let folder = sources::list(root).unwrap().pop().unwrap().id;
    inventory::control::enqueue(root, &folder).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    h.inv.monitor.set(|s| {
        s.power = PowerSource::Ac;
        s.battery_percent = Some(90.0);
    });
    assert!(
        wait_until(LONG, || settled(&ws)),
        "the rescan never finished"
    );
    drop(ws);

    let ev = events(&env);
    let at = |from, want: &dyn Fn(&Event) -> bool, what: &str| {
        find(&ev, from, want).unwrap_or_else(|| panic!("no {what} after {from} in {ev:#?}"))
    };
    let started = at(0, &|e| matches!(e, Event::AppStarted { .. }), "start");
    let opened = at(
        started,
        &|e| matches!(e, Event::LibraryOpened { created: true, .. }),
        "open",
    );
    let scan = at(
        opened,
        &|e| {
            matches!(
                e,
                Event::JobStarted {
                    kind: JobKind::Scan
                }
            )
        },
        "scan",
    );
    let scanned = at(
        scan,
        &|e| matches!(e, Event::JobEnded { kind: JobKind::Scan, state: Some(JobState::Completed), items, .. } if *items >= 5),
        "the scan's end",
    );
    let read = at(
        opened,
        &|e| {
            matches!(
                e,
                Event::JobStarted {
                    kind: JobKind::Extract
                }
            )
        },
        "reading",
    );
    at(
        read,
        &|e| {
            matches!(
                e,
                Event::WorkerStarted {
                    role: WorkerRole::Read
                }
            )
        },
        "a worker",
    );
    at(
        read,
        &|e| {
            matches!(
                e,
                Event::WorkerCrashed {
                    role: WorkerRole::Read,
                    crash: Crash::Exited
                }
            )
        },
        "the crash",
    );
    at(
        read,
        &|e| {
            matches!(
                e,
                Event::JobEnded {
                    kind: JobKind::Extract,
                    ..
                }
            )
        },
        "reading's end",
    );
    let paused = at(
        scanned,
        &|e| {
            matches!(
                e,
                Event::JobPaused {
                    kind: JobKind::Scan,
                    by: PauseBy::Policy,
                    reason: Some(PauseReason::BatteryLow)
                }
            )
        },
        "the pause",
    );
    at(
        paused,
        &|e| {
            matches!(
                e,
                Event::JobResumed {
                    kind: JobKind::Scan,
                    by: PauseBy::Policy
                }
            )
        },
        "the resume",
    );
    at(
        0,
        &|e| {
            matches!(
                e,
                Event::PolicyChanged {
                    mode: PolicyMode::Paused,
                    ..
                }
            )
        },
        "the policy change",
    );
    assert_eq!(
        ev.iter()
            .filter(|e| matches!(e, Event::JobPaused { .. }))
            .count(),
        1,
        "a pause is logged once, however often it is re-checked"
    );
    assert!(
        matches!(ev.last(), Some(Event::LibraryClosed { .. })),
        "{:?}",
        ev.last()
    );

    let text = log_text(&env);
    for word in [
        "crash-exit",
        "book ",
        "Books",
        "AtlasData",
        env.base.to_str().unwrap(),
    ] {
        assert!(!text.contains(word), "the log names {word:?}");
    }
    assert!(WorkerStats::get(&h.deps.stats.kills) >= 1);
}

/// P4-01: nothing is written with no library open, and the log stops
/// when its library closes.
#[test]
fn p4_01_only_an_open_library_has_a_log() {
    let dir = tempfile::tempdir().unwrap();
    let d = Diagnostics::new();
    d.record(Event::LogCleared);
    {
        let _guard = d.attach(dir.path());
        d.record(Event::ReportSaved { bytes: 7 });
    }
    d.record(Event::ReportSaved { bytes: 8 });
    d.flush();
    let lines: Vec<Event> = diagnostics::tail(&dir.path().join(LOGS_DIR), 100)
        .into_iter()
        .map(|l: LogLine| l.event)
        .collect();
    assert!(matches!(lines[0], Event::AppStarted { .. }));
    assert_eq!(lines[1], Event::ReportSaved { bytes: 7 });
    assert!(matches!(lines[2], Event::LibraryClosed { .. }));
    assert_eq!(lines.len(), 3, "{lines:?}");
}

/// P4-01: a log that can't be written stops itself; the work goes on.
#[cfg(unix)]
#[test]
fn p4_01_a_log_that_cant_be_written_never_stops_the_work() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let ws = read_library(&env, &h, 2, &[]);
    drop(ws);
    let logs = data(&env).join(LOGS_DIR);
    for e in fs::read_dir(&logs).unwrap().flatten() {
        fs::remove_file(e.path()).unwrap();
    }
    fs::set_permissions(&logs, fs::Permissions::from_mode(0o500)).unwrap();

    let mut ws = env.workspace_with_extraction(&h);
    ws.startup();
    let root = ws.require_open().unwrap();
    let folder = sources::list(root).unwrap().pop().unwrap().id;
    inventory::control::enqueue(root, &folder).unwrap();
    assert!(
        wait_until(LONG, || settled(&ws)),
        "the rescan never finished"
    );
    assert!(ws.require_open().unwrap().diagnostics().is_broken());
    let status = diagnostics::status(&data(&env), ws.diagnostics());
    assert!(status.log_stopped);
    drop(ws);
    fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(fs::read_dir(&logs).unwrap().count(), 0);
}

fn count(c: &Connection, sql: &str) -> u64 {
    c.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap() as u64
}

fn save(ws: &Workspace) -> (PathBuf, Report) {
    let root = ws.require_open().unwrap();
    let store = root.knowledge();
    let d = root.diagnostics().clone();
    d.flush();
    let inputs = Inputs {
        root: root.path().to_path_buf(),
        device: None,
        settings: root.settings().clone(),
        storage: storage::usage(root.path(), ws.volumes().as_ref()).ok(),
        knowledge: KnowledgeSummary {
            state: store.state(),
            status: (store.state() == KnowledgeState::Ready).then(|| store.status()),
        },
        dropped: d.dropped(),
        saved_at: "2026-11-12T10:00:00Z".into(),
    };
    let saved = report::save(&inputs).unwrap();
    let bytes = fs::read(&saved.path).unwrap();
    assert_eq!(bytes.len() as u64, saved.bytes);
    let report: Report = serde_json::from_slice(&bytes).expect("the report reads back");
    (saved.path, report)
}

/// P4-03: the report is written under `exports/diagnostics` only, reads
/// back as itself, and its counts are the catalog's and the store's. With
/// the store damaged it is still saved, with the store's state only.
#[test]
fn p4_03_the_report_counts_what_the_library_holds() {
    let env = Env::new();
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let ws = read_library(
        &env,
        &h,
        3,
        &[
            ("pdf/scanned/ia-alice-1894.pdf", "scan.pdf"),
            ("pdf/generated/book.pdf", "book.pdf"),
        ],
    );
    let before: Vec<PathBuf> = walk(&data(&env));
    let (path, r) = save(&ws);
    assert!(path.starts_with(data(&env).join("exports/diagnostics")));
    let after: Vec<PathBuf> = walk(&data(&env));
    let new: Vec<_> = after.iter().filter(|p| !before.contains(p)).collect();
    assert!(
        new.iter()
            .all(|p| p.starts_with(data(&env).join("exports/diagnostics"))
                || p.starts_with(data(&env).join(LOGS_DIR))),
        "{new:?}"
    );

    let c = Connection::open(data(&env).join("library/catalog.sqlite")).unwrap();
    let lib = &r.library;
    assert_eq!(lib.folders, 1);
    assert_eq!(
        lib.files_by_availability.values().sum::<u64>(),
        count(&c, "SELECT COUNT(*) FROM file_instances")
    );
    assert_eq!(lib.passages, count(&c, "SELECT COUNT(*) FROM passages"));
    assert_eq!(lib.documents_by_state.get("needs_ocr"), Some(&1));
    assert_eq!(lib.ocr.pdfs_needing_ocr, 1);
    assert_eq!((lib.ocr.pages, lib.ocr.pages_with_text), (4, 0));
    assert_eq!(lib.ocr.uncounted_pdfs, 0);
    assert_eq!(
        lib.files_by_format["pdf"].bytes,
        count(
            &c,
            "SELECT SUM(size_bytes) FROM file_instances WHERE file_type = 'pdf'"
        )
    );
    assert_eq!(lib.jobs.get("scan.completed"), Some(&1));
    assert_eq!(r.versions.catalog_schema, SUPPORTED_SCHEMA_VERSION);
    assert_eq!(r.knowledge.state, KnowledgeState::Ready);
    assert_eq!(r.knowledge.status.as_ref().unwrap().trails, 0);
    assert!(r.storage.as_ref().unwrap().log_bytes > 0);
    assert!(!r.events.is_empty(), "the log's last events are in it");
    drop(c);
    drop(ws);

    // The store damaged: the report still saves.
    let store = data(&env).join("library/knowledge.sqlite");
    for suffix in ["-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{suffix}", store.display()));
    }
    fs::write(&store, b"not a database".repeat(100)).unwrap();
    let mut ws = env.workspace_with_extraction(&h);
    ws.startup();
    let (_, r) = save(&ws);
    assert_eq!(r.knowledge.state, KnowledgeState::Damaged);
    assert!(r.knowledge.status.is_none());
    let reports = report::saved(&data(&env));
    assert_eq!(reports.len(), 2, "a second report beside the first");
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

/// P4-04: a worker of another protocol ends reading for good, with a code
/// the status line can explain; it isn't restarted over and over, and no
/// file's status changes.
#[test]
fn p4_04_reading_stops_for_a_worker_of_another_protocol() {
    let env = Env::new();
    let books = env.base.join("Books");
    fixture(&books, 3);
    let h = ExtractionHarness::new(fast_extraction(), worker().env(TEST_PROTOCOL_ENV, "1"));
    let mut ws = env.workspace_with_extraction(&h);
    ws.choose(&data(&env)).unwrap();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &books).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    let overview = || {
        ws.require_open()
            .unwrap()
            .reader()
            .read(|c| db::overview(c, None))
            .unwrap()
            .unwrap()
    };
    assert!(
        wait_until(LONG, || overview()
            .roots
            .iter()
            .any(|r| r.failed_because.as_deref() == Some("worker_protocol"))),
        "reading never stopped: {:?}",
        overview()
    );
    let ov = overview();
    assert_eq!(
        ov.counts.pending, ov.counts.total,
        "no file was read or marked"
    );
    assert!(WorkerStats::get(&h.deps.stats.spawns) <= 2);
    drop(ws);
    let events = events(&env);
    assert!(events.iter().any(|e| matches!(
        e,
        Event::WorkerRefused {
            role: WorkerRole::Read,
            protocol: 1
        }
    )));
    // Slice 4.1.1: the job's end says why it failed.
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::JobEnded {
                kind: JobKind::Extract,
                state: Some(JobState::FailedTerminal),
                reason: Some(FailureReason::WorkerProtocol),
                ..
            }
        )),
        "{events:?}"
    );
}

/// What recording costs (docs/benchmarks.md, Slice 4.1): 100,000 events
/// through the queue to the file, and how many a reading session writes.
#[test]
#[ignore = "a measurement: cargo test --release --test diagnostics -- --ignored --nocapture"]
fn measure_the_cost_of_recording() {
    let dir = tempfile::tempdir().unwrap();
    let d = Diagnostics::new();
    let guard = d.attach(dir.path());
    let n = 100_000u64;
    let started = std::time::Instant::now();
    // In bursts the queue holds, so every event is written; a longer
    // burst would drop the rest and count them.
    let mut queued = Duration::ZERO;
    for i in 0..n {
        let t = std::time::Instant::now();
        d.record(Event::JobEnded {
            kind: JobKind::Extract,
            state: Some(JobState::Completed),
            ms: i,
            items: i,
            errors: 0,
            reason: None,
        });
        queued += t.elapsed();
        if i % 512 == 511 {
            d.flush();
        }
    }
    d.flush();
    let written = started.elapsed();
    drop(guard);
    let files: Vec<_> = fs::read_dir(dir.path().join(LOGS_DIR))
        .unwrap()
        .flatten()
        .collect();
    let bytes: u64 = files.iter().map(|e| e.metadata().unwrap().len()).sum();
    let lines = diagnostics::tail(&dir.path().join(LOGS_DIR), usize::MAX >> 1).len();
    println!(
        "diagnostics-cost {}",
        serde_json::json!({
            "events": n,
            "dropped": d.dropped(),
            "lines": lines,
            "recordNsPerEvent": queued.as_nanos() as u64 / n,
            "writtenNsPerEvent": written.as_nanos() as u64 / n,
            "bytesPerEvent": bytes / n,
        })
    );
}
