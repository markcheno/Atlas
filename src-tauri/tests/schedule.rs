//! Slice 4.6: when background work runs (phase4-spec §5.4, §6.3, §7.5;
//! P4-13, P4-14, P4-25). The owner's schedule (Automatic, Only while
//! charging, Only between two times, Paused), full speed and its ends, and
//! the busy-computer rule, on real jobs with a fake device, a settable
//! clock and a fake processor reading.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_lib::domain::dto::{
    BackgroundMode, FullSpeedBlock, PolicyMode, PowerSource, ResourceStatus, Settings,
    ThemePreference,
};
use atlas_lib::error::AtlasError;
use atlas_lib::infrastructure::clock::TestClock;
use atlas_lib::services::diagnostics::event::FullSpeedEnd;
use atlas_lib::services::diagnostics::{self, Event, LOGS_DIR};
use atlas_lib::services::embedding::triggers;
use atlas_lib::services::scheduler::busy::{BusyRule, CpuProbe};
use atlas_lib::services::scheduler::watch::set_full_speed;
use atlas_lib::services::system::ResourceGovernor;
use atlas_lib::services::workspace::{OpenRoot, Workspace, layout};
use atlas_lib::services::{inventory, sources};
use rusqlite::OptionalExtension;
use time::UtcOffset;
use time::macros::datetime;

mod common;
use common::embedding::{EmbedHarness, fast_embedding};
use common::{Env, wait_until};

const LONG: Duration = Duration::from_secs(120);
const GIB: u64 = 1024 * 1024 * 1024;

/// A processor reading the test sets.
#[derive(Clone, Default)]
struct FakeCpu(Arc<Mutex<Option<f32>>>);

impl FakeCpu {
    fn set(&self, share: f32) {
        *self.0.lock().unwrap() = Some(share);
    }
}

impl CpuProbe for FakeCpu {
    fn sample(&mut self) -> Option<f32> {
        *self.0.lock().unwrap()
    }
}

struct Lib {
    env: Env,
    h: EmbedHarness,
    clock: Arc<TestClock>,
    cpu: FakeCpu,
    ws: Workspace,
    books: PathBuf,
}

impl Lib {
    /// An empty library, opened at noon UTC, with six generated books in a
    /// folder not yet added.
    fn new() -> Self {
        Self::with(fast_embedding(), 6)
    }

    /// As [`Lib::new`], with this embedding configuration and number of books.
    fn with(config: atlas_lib::services::embedding::job::EmbedConfig, files: u32) -> Self {
        let h = EmbedHarness::new(config);
        let clock = Arc::new(TestClock::at(datetime!(2026-11-16 12:00 UTC)));
        let cpu = FakeCpu::default();
        {
            let mut g = h.ex.inv.deps.governor.lock().unwrap();
            g.clock = clock.clone();
            g.cpu = Box::new(cpu.clone());
            g.busy = BusyRule::new(Duration::from_millis(200), Duration::from_millis(10));
        }
        let env = Env::new();
        let mut ws = env.workspace_with_embedding(&h).with_clock(clock.clone());
        h.ex.inv.deps.governor.lock().unwrap().diagnostics = Some(ws.diagnostics().clone());
        ws.choose(&env.base.join("AtlasData")).unwrap();
        let books = env.base.join("Books");
        fixture(&books, files);
        Self {
            env,
            h,
            clock,
            cpu,
            ws,
            books,
        }
    }

    fn root(&self) -> &OpenRoot {
        self.ws.require_open().unwrap()
    }

    fn governor(&self) -> &Mutex<ResourceGovernor> {
        &self.h.ex.inv.deps.governor
    }

    fn status(&self) -> ResourceStatus {
        let path = self.root().path().to_path_buf();
        self.governor().lock().unwrap().status(Some(&path))
    }

    fn background(
        &mut self,
        mode: BackgroundMode,
        from: &str,
        to: &str,
    ) -> Result<Settings, AtlasError> {
        let mut s = self.ws.settings().unwrap();
        s.background.mode = mode;
        s.background.from = from.into();
        s.background.to = to.into();
        self.ws.update_settings(s)
    }

    fn mode(&mut self, mode: BackgroundMode) {
        self.background(mode, "22:00", "07:00").unwrap();
    }

    fn add(&self) {
        let added = sources::add(self.root(), &self.books).unwrap();
        inventory::control::enqueue(self.root(), &added.id).unwrap();
    }

    /// The newest job of `kind`: its state, pause reason and throttle reason.
    fn job(&self, kind: &str) -> Option<(String, Option<String>, Option<String>)> {
        let kind = kind.to_owned();
        self.root()
            .catalog()
            .call(move |c| {
                c.query_row(
                    "SELECT state, pause_reason, throttle_reason FROM jobs
                     WHERE kind = ?1 ORDER BY rowid DESC LIMIT 1",
                    [kind],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .unwrap()
            })
            .unwrap()
    }

    fn wait(&self, kind: &str, state: &str, reason: Option<&str>) {
        assert!(
            wait_until(LONG, || self
                .job(kind)
                .is_some_and(|(s, r, _)| s == state && r.as_deref() == reason)),
            "{kind}: wanted {state} {reason:?}, found {:?}",
            self.job(kind)
        );
    }

    fn waiting(&self) -> i64 {
        self.root()
            .catalog()
            .call(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM documents d WHERE d.state = 'done' AND NOT EXISTS
                         (SELECT 1 FROM document_embeddings de WHERE de.document_id = d.id)",
                    [],
                    |r| r.get(0),
                )
                .unwrap()
            })
            .unwrap()
    }

    /// Cataloged and read.
    fn read(&self) {
        self.wait("scan", "completed", None);
        self.wait("extract", "completed", None);
    }

    fn set_role(&self) {
        self.root()
            .catalog()
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO provider_profiles (id, kind, label, base_url,
                         insecure_http_acknowledged, created_at, updated_at)
                     VALUES ('svc', 'openai_compatible', 'Test', 'http://localhost:9/v1', 1,
                             't', 't');
                     INSERT INTO model_roles (role, provider_profile_id, model_id,
                         dimensions, tested_at, updated_at)
                     VALUES ('embedding', 'svc', 'test-embed', 64, 't', 't');",
                )
                .unwrap();
            })
            .unwrap();
        triggers::changed(self.root()).unwrap();
    }

    fn full_speed(&self, on: bool) -> Result<ResourceStatus, AtlasError> {
        set_full_speed(self.root(), self.governor(), on)
    }

    fn full_speed_on(&self) -> bool {
        self.governor().lock().unwrap().full_speed()
    }

    fn events(&self) -> Vec<Event> {
        diagnostics::tail(&self.env.base.join("AtlasData").join(LOGS_DIR), 100_000)
            .into_iter()
            .map(|l| l.event)
            .collect()
    }

    fn ended(&self) -> Vec<FullSpeedEnd> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                Event::FullSpeed {
                    on: false, ended, ..
                } => ended,
                _ => None,
            })
            .collect()
    }
}

fn fixture(dir: &Path, files: u32) {
    let out = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
        .arg(dir)
        .args(["--files", &files.to_string(), "--text-kb", "2"])
        .output()
        .expect("node runs the fixture generator");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// P4-13: Paused holds cataloging and indexing, each saying why, and
/// Automatic lets them go on.
#[test]
fn p4_13_paused_holds_the_jobs_and_automatic_lets_them_go() {
    let mut lib = Lib::new();
    lib.mode(BackgroundMode::Paused);
    lib.add();
    lib.wait("scan", "paused_policy", Some("paused_by_user"));
    let status = lib.status();
    assert_eq!(status.mode, BackgroundMode::Paused);
    assert_eq!(status.full_speed.blocked, Some(FullSpeedBlock::Paused));
    assert!(matches!(
        lib.full_speed(true),
        Err(AtlasError::BackgroundPaused)
    ));

    lib.mode(BackgroundMode::Automatic);
    lib.read();

    lib.mode(BackgroundMode::Paused);
    lib.set_role();
    lib.wait("embed", "paused_policy", Some("paused_by_user"));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(lib.h.script.calls(), 0, "nothing sent while Paused");

    lib.mode(BackgroundMode::Automatic);
    lib.wait("embed", "completed", None);
    assert_eq!(lib.waiting(), 0);
}

/// P4-13: Only between two times, by the local clock, across midnight.
#[test]
fn p4_13_hours_hold_work_outside_the_window() {
    let mut lib = Lib::new();
    lib.clock.set(datetime!(2026-11-16 21:58 UTC));
    lib.mode(BackgroundMode::Hours);
    lib.add();
    lib.wait("scan", "paused_policy", Some("outside_hours"));
    assert_eq!(lib.status().waiting_until.as_deref(), Some("22:00"));

    lib.clock.set(datetime!(2026-11-16 22:00 UTC));
    lib.read();
    assert_eq!(lib.status().waiting_until, None);

    // Across midnight, and by the local offset, not UTC.
    for (utc, offset, running) in [
        (datetime!(2026-11-17 06:59 UTC), 0, true),
        (datetime!(2026-11-17 07:00 UTC), 0, false),
        (datetime!(2026-11-17 03:00 UTC), -6, false), // 21:00 local
        (datetime!(2026-11-17 05:00 UTC), -6, true),  // 23:00 local
    ] {
        lib.clock.set(utc);
        lib.clock
            .set_offset(UtcOffset::from_hms(offset, 0, 0).unwrap());
        let paused = lib.status().policy.mode == PolicyMode::Paused;
        assert_eq!(!paused, running, "{utc} at {offset}");
    }

    // Two equal times can't be saved.
    assert!(matches!(
        lib.background(BackgroundMode::Hours, "08:00", "08:00"),
        Err(AtlasError::InvalidHours)
    ));
    assert!(matches!(
        lib.background(BackgroundMode::Hours, "8", "09:00"),
        Err(AtlasError::InvalidHours)
    ));
}

/// P4-13: Only while charging waits for power on a laptop, never on a
/// desktop.
#[test]
fn p4_13_only_while_charging_waits_for_power() {
    let mut lib = Lib::new();
    let monitor = lib.h.ex.inv.monitor.clone();
    monitor.set(|s| {
        s.power = PowerSource::Battery;
        s.battery_present = Some(true);
        s.battery_percent = Some(80.0);
    });
    lib.mode(BackgroundMode::Charging);
    lib.add();
    lib.wait("scan", "paused_policy", Some("not_charging"));
    assert_eq!(
        lib.status().full_speed.blocked,
        Some(FullSpeedBlock::OnBattery)
    );
    assert!(matches!(
        lib.full_speed(true),
        Err(AtlasError::FullSpeedOnBattery)
    ));

    monitor.set(|s| s.power = PowerSource::Ac);
    lib.read();

    // A desktop with no battery is never waiting for power, even when the
    // probe can't name the power source.
    monitor.set(|s| {
        s.power = PowerSource::Unknown;
        s.battery_present = Some(false);
        s.battery_percent = None;
    });
    assert_eq!(lib.status().policy.pause_reason, None);
}

/// P4-14: full speed runs the jobs without rests until the work runs out;
/// Stop, unplugging and closing end it; hard pauses still hold.
#[test]
fn p4_14_full_speed_until_the_work_is_done() {
    let mut lib = Lib::new();
    lib.add();
    lib.read();
    assert!(
        matches!(lib.full_speed(true), Err(AtlasError::NothingWaiting)),
        "nothing waiting"
    );

    lib.h.script.with(|s| s.delay = Duration::from_millis(40));
    lib.set_role();
    lib.wait("embed", "running", None);
    let status = lib.full_speed(true).unwrap();
    assert!(status.full_speed.on && status.policy.full_speed);
    assert_eq!(status.policy.max_metadata_ops_per_second, None);

    // A hard pause still holds every job, and full speed waits it out.
    let monitor = lib.h.ex.inv.monitor.clone();
    monitor.set(|s| {
        s.data_volume_free_bytes = Some(GIB);
        s.data_volume_total_bytes = Some(1000 * GIB);
    });
    lib.wait("embed", "paused_policy", Some("low_disk_space"));
    assert!(lib.full_speed_on());
    monitor.set(|s| {
        s.data_volume_free_bytes = None;
        s.data_volume_total_bytes = None;
    });
    lib.wait("embed", "running", None);

    // Unplugging ends it.
    monitor.set(|s| {
        s.power = PowerSource::Battery;
        s.battery_present = Some(true);
        s.battery_percent = Some(80.0);
    });
    assert!(wait_until(LONG, || !lib.full_speed_on()));
    monitor.set(|s| s.power = PowerSource::Ac);

    // Stop ends it.
    lib.wait("embed", "running", None);
    lib.full_speed(true).unwrap();
    let status = lib.full_speed(false).unwrap();
    assert!(!status.full_speed.on && !status.policy.full_speed);

    // Once nothing is waiting, it ends by itself.
    lib.full_speed(true).unwrap();
    lib.h.script.with(|s| s.delay = Duration::ZERO);
    lib.wait("embed", "completed", None);
    assert!(
        wait_until(LONG, || !lib.full_speed_on()),
        "full speed ended when the work ran out"
    );
    assert_eq!(lib.waiting(), 0);

    // Paused ends it too.
    lib.h.script.with(|s| s.delay = Duration::from_millis(40));
    fixture(&lib.books.join("More"), 3);
    let root_id: String = lib
        .root()
        .catalog()
        .call(|c| {
            c.query_row("SELECT id FROM source_roots", [], |r| r.get(0))
                .unwrap()
        })
        .unwrap();
    inventory::control::enqueue(lib.root(), &root_id).unwrap();
    assert!(wait_until(LONG, || lib.full_speed(true).is_ok()));
    lib.mode(BackgroundMode::Paused);
    assert!(!lib.full_speed_on());
    lib.mode(BackgroundMode::Automatic);

    // Closing ends it.
    assert!(wait_until(LONG, || lib.full_speed(true).is_ok()));
    lib.ws.shutdown();
    assert!(!lib.governor().lock().unwrap().full_speed());

    let ended = lib.ended();
    for want in [
        FullSpeedEnd::Unplugged,
        FullSpeedEnd::Stopped,
        FullSpeedEnd::Done,
        FullSpeedEnd::Paused,
        FullSpeedEnd::Closed,
    ] {
        assert!(ended.contains(&want), "{want:?} in {ended:?}");
    }
}

/// P4-14: other applications' processor use slows work; it returns once
/// they ease off; full speed overrides it.
#[test]
fn p4_14_other_apps_slow_work_and_it_returns() {
    let lib = Lib::new();
    lib.add();
    lib.read();
    lib.h.script.with(|s| s.delay = Duration::from_millis(30));
    lib.cpu.set(0.9);
    lib.set_role();
    assert!(
        wait_until(LONG, || lib.job("embed").is_some_and(|(s, _, t)| s
            == "running"
            && t.as_deref() == Some("other_apps"))),
        "{:?}",
        lib.job("embed")
    );
    lib.full_speed(true).unwrap();
    assert!(wait_until(LONG, || lib
        .job("embed")
        .is_some_and(|(s, _, t)| s != "running" || t.is_none())));
    lib.full_speed(false).unwrap();

    // Between the thresholds it stays slowed; under 40% it returns.
    lib.cpu.set(0.5);
    std::thread::sleep(Duration::from_millis(300));
    assert!(lib.status().policy.throttle_reason.is_some());
    lib.cpu.set(0.1);
    assert!(wait_until(LONG, || lib
        .status()
        .policy
        .throttle_reason
        .is_none()));
}

/// P4-25: settings files from 0.4.0 and from Slice 4.5 are valid, with the
/// new fields at their defaults; values this build doesn't know read as
/// defaults without resetting the rest.
#[test]
fn p4_25_older_and_unknown_settings_read_as_defaults() {
    let env = Env::new();
    let mut ws = env.workspace();
    let data = env.base.join("AtlasData");
    ws.choose(&data).unwrap();
    let file = data.join(layout::SETTINGS);

    let read = |ws: &mut Workspace, json: &str| {
        fs::write(&file, json).unwrap();
        ws.reopen();
        ws.settings().unwrap()
    };

    // 0.4.0: no background section at all.
    let s = read(
        &mut ws,
        r#"{"schemaVersion":1,"appearance":{"theme":"dark"},
            "privacy":{"allowRemoteEmbedding":false,"disableExternalAi":false}}"#,
    );
    assert_eq!(s.appearance.theme, ThemePreference::Dark);
    assert_eq!(s.background, Default::default());
    assert_eq!(
        (s.background.from.as_str(), s.background.to.as_str()),
        ("22:00", "07:00")
    );

    // Slice 4.5: the folder check only.
    let s = read(
        &mut ws,
        r#"{"schemaVersion":1,"background":{"checkFolders":"daily"}}"#,
    );
    assert_eq!(s.background.mode, BackgroundMode::Automatic);
    assert_eq!(
        s.background.check_folders,
        atlas_lib::domain::dto::CheckFolders::Daily
    );

    // Values this build doesn't know.
    let s = read(
        &mut ws,
        r#"{"schemaVersion":1,"appearance":{"theme":"dark"},
            "background":{"mode":"sometimes","from":"25:00","to":7,"checkFolders":"daily"}}"#,
    );
    assert_eq!(s.appearance.theme, ThemePreference::Dark);
    assert_eq!(s.background.mode, BackgroundMode::Automatic);
    assert_eq!(
        (s.background.from.as_str(), s.background.to.as_str()),
        ("22:00", "07:00")
    );
    assert_eq!(
        s.background.check_folders,
        atlas_lib::domain::dto::CheckFolders::Daily
    );

    // A hand-edited file with equal hours, not in use, doesn't stop other
    // settings from being saved; choosing them does.
    let mut s = read(
        &mut ws,
        r#"{"schemaVersion":1,"background":{"mode":"automatic","from":"08:00","to":"08:00"}}"#,
    );
    s.appearance.theme = ThemePreference::Light;
    ws.update_settings(s.clone()).unwrap();
    s.background.mode = BackgroundMode::Hours;
    assert!(matches!(
        ws.update_settings(s),
        Err(AtlasError::InvalidHours)
    ));
}

/// What the real processor reading costs and reports (`just bench-busy`):
/// idle, with Atlas's own threads busy (they must not count), and with
/// busy processes of another program (they must). Unix only: the busy
/// processes are `sh` loops.
#[cfg(unix)]
#[test]
#[ignore = "a measurement: just bench-busy"]
fn measure_the_cpu_probe() {
    use atlas_lib::infrastructure::cpu_probe::OsCpuProbe;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    let cores = std::thread::available_parallelism().unwrap().get();
    let half = (cores / 2).max(1);
    let mut probe = OsCpuProbe::new();
    let mut costs = Vec::new();
    let mut read = |probe: &mut OsCpuProbe| {
        let started = Instant::now();
        let share = probe.sample();
        costs.push(started.elapsed().as_secs_f64() * 1000.0);
        share
    };
    read(&mut probe);
    std::thread::sleep(Duration::from_secs(5));
    let idle = read(&mut probe).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let threads: Vec<_> = (0..half)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut x = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                }
                x
            })
        })
        .collect();
    read(&mut probe);
    std::thread::sleep(Duration::from_secs(5));
    let own_busy = read(&mut probe).unwrap();
    stop.store(true, Ordering::Relaxed);
    for t in threads {
        t.join().unwrap();
    }

    let mut children: Vec<_> = (0..half)
        .map(|_| {
            Command::new("sh")
                .args(["-c", "while :; do :; done"])
                .spawn()
                .unwrap()
        })
        .collect();
    read(&mut probe);
    std::thread::sleep(Duration::from_secs(5));
    let others_busy = read(&mut probe).unwrap();
    for c in &mut children {
        let _ = c.kill();
        let _ = c.wait();
    }

    costs.sort_by(f64::total_cmp);
    println!(
        "cpu-probe cores={cores} busy_processes={half} idle={idle:.3} own_threads_busy={own_busy:.3} \
         other_processes_busy={others_busy:.3} expected_other={:.3} sample_ms_p50={:.1} sample_ms_max={:.1}",
        half as f32 / cores as f32,
        costs[costs.len() / 2],
        costs[costs.len() - 1],
    );
    assert!(own_busy < idle + 0.15, "Atlas's own threads don't count");
    assert!(
        others_busy > idle + 0.5 * half as f32 / cores as f32,
        "other processes do"
    );
}

/// What full speed does to indexing meaning (`just bench-busy`): the same
/// books embedded through a fake service that takes 20 ms a request, at
/// the Run duty cycle (0.5) and at full speed.
#[test]
#[ignore = "a measurement: just bench-busy"]
fn measure_full_speed_against_the_run_duty_cycle() {
    use atlas_lib::services::embedding::job::EmbedConfig;
    use std::time::Instant;

    let mut rates = Vec::new();
    for full in [false, true] {
        let config = EmbedConfig {
            unthrottled: false,
            ..fast_embedding()
        };
        let lib = Lib::with(config, 40);
        lib.add();
        lib.read();
        lib.h.script.with(|s| s.delay = Duration::from_millis(20));
        let started = Instant::now();
        lib.set_role();
        if full {
            assert!(wait_until(LONG, || lib.full_speed(true).is_ok()));
        }
        lib.wait("embed", "completed", None);
        let secs = started.elapsed().as_secs_f64();
        let chunks = lib.h.script.answered().len();
        rates.push(chunks as f64 / secs);
        println!(
            "full-speed full={full} chunks={chunks} secs={secs:.2} chunks_per_s={:.1}",
            chunks as f64 / secs
        );
    }
    println!("full-speed ratio={:.2}", rates[1] / rates[0]);
    assert!(rates[1] > rates[0] * 1.5, "full speed rests no more");
}
