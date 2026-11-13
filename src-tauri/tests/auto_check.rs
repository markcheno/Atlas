//! Slice 4.5: Atlas checks its folders by itself (phase4-spec §7.4, P4-12;
//! ADR 0022): at launch, on its interval, and when a drive returns; never
//! while a folder already has a scan, never when the setting is "Only when
//! I ask", never for a folder that is away. Time is a settable clock, and
//! the check is called directly, as the knowledge store's tests call its
//! own `*_if_due`.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use atlas_lib::domain::dto::{CheckFolders, JobState, PauseReason, PowerSource};
use atlas_lib::infrastructure::clock::{Clock, TestClock};
use atlas_lib::services::inventory::auto::{CheckState, check_if_due};
use atlas_lib::services::inventory::{InventoryConfig, control};
use atlas_lib::services::sources;
use atlas_lib::services::workspace::Workspace;

mod common;
use common::{Env, InventoryHarness, fast_config, wait_until};

const LONG: Duration = Duration::from_secs(120);

struct Lib {
    env: Env,
    h: InventoryHarness,
    clock: Arc<TestClock>,
    ws: Workspace,
}

impl Lib {
    /// With the workspace's own checker slowed to once an hour, so only
    /// the test's calls check.
    fn new() -> Self {
        Self::with(InventoryConfig {
            check_every: Duration::from_secs(3600),
            ..fast_config()
        })
    }

    fn with(config: InventoryConfig) -> Self {
        let env = Env::new();
        let h = InventoryHarness::new(config);
        let clock = Arc::new(TestClock::starting_now());
        let mut ws = env
            .workspace_with_inventory(h.deps.clone())
            .with_clock(clock.clone());
        ws.choose(&env.base.join("AtlasData")).unwrap();
        Self { env, h, clock, ws }
    }

    fn add(&self, path: &Path) -> (String, String) {
        let root = self.ws.require_open().unwrap();
        let added = sources::add(root, path).unwrap();
        let job = control::enqueue(root, &added.id).unwrap();
        (added.id, job)
    }

    fn check(&self, setting: CheckFolders, state: &mut CheckState) -> (Vec<String>, Vec<String>) {
        let root = self.ws.require_open().unwrap();
        let done = check_if_due(&root.catalog().clone(), &*self.clock, setting, state).unwrap();
        root.wake_jobs();
        (done.queued, done.resumed)
    }

    fn state(&self, job: &str) -> String {
        let id = job.to_owned();
        self.ws
            .require_open()
            .unwrap()
            .catalog()
            .call(move |c| {
                c.query_row("SELECT state FROM jobs WHERE id = ?1", [id], |r| r.get(0))
                    .unwrap()
            })
            .unwrap()
    }

    fn wait(&self, job: &str, want: &str) {
        assert!(
            wait_until(LONG, || self.state(job) == want),
            "{job} never reached {want}; now {}",
            self.state(job)
        );
    }

    /// The folder's scan jobs as (state, origin), oldest first.
    fn scans(&self, root_id: &str) -> Vec<(String, Option<String>)> {
        let id = root_id.to_owned();
        self.ws
            .require_open()
            .unwrap()
            .catalog()
            .call(move |c| {
                c.prepare(
                    "SELECT state, origin FROM jobs WHERE source_root_id = ?1 AND kind = 'scan'
                     ORDER BY rowid",
                )
                .unwrap()
                .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
            })
            .unwrap()
    }

    fn latest_scan(&self, root_id: &str) -> String {
        let id = root_id.to_owned();
        self.ws
            .require_open()
            .unwrap()
            .catalog()
            .call(move |c| {
                c.query_row(
                    "SELECT id FROM jobs WHERE source_root_id = ?1 AND kind = 'scan'
                     ORDER BY rowid DESC LIMIT 1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap()
            })
            .unwrap()
    }
}

fn books(lib: &Lib, name: &str, files: usize) -> std::path::PathBuf {
    let dir = lib.env.base.join(name);
    fs::create_dir_all(&dir).unwrap();
    for i in 0..files {
        fs::write(dir.join(format!("{i:03}.pdf")), b"%PDF").unwrap();
    }
    dir
}

fn hours(h: i64) -> time::Duration {
    time::Duration::hours(h)
}

#[test]
fn p4_12_a_folder_is_checked_when_its_interval_has_passed_and_not_before() {
    let lib = Lib::new();
    let (root_id, first) = lib.add(&books(&lib, "Books", 3));
    lib.wait(&first, "completed");
    let mut state = CheckState::default();

    // Just scanned: nothing is due.
    assert_eq!(lib.check(CheckFolders::SixHours, &mut state).0, Vec::<String>::new());
    lib.clock.advance(hours(5));
    assert!(lib.check(CheckFolders::SixHours, &mut state).0.is_empty());
    // Past six hours: one automatic check.
    lib.clock.advance(hours(2));
    assert_eq!(lib.check(CheckFolders::SixHours, &mut state).0, vec![root_id.clone()]);
    // Never two scans of one folder.
    assert!(lib.check(CheckFolders::SixHours, &mut state).0.is_empty());
    let auto = lib.latest_scan(&root_id);
    lib.wait(&auto, "completed");
    assert_eq!(
        lib.scans(&root_id),
        vec![
            ("completed".into(), Some("user".into())),
            ("completed".into(), Some("automatic".into()))
        ]
    );
    // Fresh again: the next is due an interval after this one.
    lib.clock.advance(hours(1));
    assert!(lib.check(CheckFolders::SixHours, &mut state).0.is_empty());
    // An hourly setting makes it due already; daily doesn't.
    assert!(lib.check(CheckFolders::Daily, &mut state).0.is_empty());
    assert_eq!(lib.check(CheckFolders::Hourly, &mut state).0.len(), 1);
}

#[test]
fn p4_12_only_when_i_ask_never_checks_and_a_cancel_is_respected() {
    let lib = Lib::new();
    let dir = books(&lib, "Books", 300);
    lib.h.source.slow(Duration::from_millis(5));
    let (root_id, first) = lib.add(&dir);
    // The user cancels the first scan: nothing restarts it before the
    // interval, though the folder was never fully scanned.
    control::cancel(lib.ws.require_open().unwrap(), &first).unwrap();
    lib.wait(&first, "cancelled");
    lib.h.source.slow(Duration::ZERO);
    let mut state = CheckState::default();
    assert!(lib.check(CheckFolders::SixHours, &mut state).0.is_empty());
    lib.clock.advance(hours(48));
    assert!(lib.check(CheckFolders::Never, &mut state).0.is_empty());
    assert_eq!(lib.scans(&root_id).len(), 1);
    // The interval past, with checks allowed: one check.
    assert_eq!(lib.check(CheckFolders::Daily, &mut state).0.len(), 1);
}

#[test]
fn p4_12_a_returning_drive_is_checked_and_a_folder_away_never_is() {
    let lib = Lib::new();
    let drive = lib.env.base.join("Drive");
    let dir = books(&lib, "Drive/Books", 3);
    let (root_id, first) = lib.add(&dir);
    lib.wait(&first, "completed");
    let mut state = CheckState::default();
    assert!(lib.check(CheckFolders::SixHours, &mut state).0.is_empty());

    fs::rename(&drive, lib.env.base.join("Unplugged")).unwrap();
    lib.clock.advance(hours(7));
    // Due, but away: no scan for a folder that isn't there.
    assert!(lib.check(CheckFolders::SixHours, &mut state).0.is_empty());
    assert_eq!(lib.scans(&root_id).len(), 1);

    fs::rename(lib.env.base.join("Unplugged"), &drive).unwrap();
    lib.clock.advance(hours(0));
    // Back: checked at once.
    assert_eq!(lib.check(CheckFolders::SixHours, &mut state).0, vec![root_id.clone()]);
    let back = lib.latest_scan(&root_id);
    lib.wait(&back, "completed");

    // A drive returning is a check of its own, not only an interval's:
    // fresh, it is still checked, unless checks are off.
    fs::rename(&drive, lib.env.base.join("Unplugged")).unwrap();
    assert!(lib.check(CheckFolders::SixHours, &mut state).0.is_empty());
    fs::rename(lib.env.base.join("Unplugged"), &drive).unwrap();
    assert!(lib.check(CheckFolders::Never, &mut state).0.is_empty());
    fs::rename(&drive, lib.env.base.join("Unplugged")).unwrap();
    assert!(lib.check(CheckFolders::SixHours, &mut state).0.is_empty());
    fs::rename(lib.env.base.join("Unplugged"), &drive).unwrap();
    assert_eq!(lib.check(CheckFolders::SixHours, &mut state).0.len(), 1);
}

#[test]
fn p4_12_an_automatic_check_obeys_the_resource_policy() {
    let lib = Lib::new();
    let (root_id, first) = lib.add(&books(&lib, "Books", 3));
    lib.wait(&first, "completed");
    lib.h.monitor.set(|s| {
        s.power = PowerSource::Battery;
        s.battery_present = Some(true);
        s.battery_percent = Some(10.0);
    });
    lib.clock.advance(hours(7));
    let mut state = CheckState::default();
    assert_eq!(lib.check(CheckFolders::SixHours, &mut state).0.len(), 1);
    let auto = lib.latest_scan(&root_id);
    lib.wait(&auto, "paused_policy");
    let ov = control::overview(lib.ws.require_open().unwrap()).unwrap();
    assert_eq!(
        ov.active.unwrap().pause_reason,
        Some(PauseReason::BatteryLow)
    );
    // Still one scan for the folder, paused, not a second.
    assert!(lib.check(CheckFolders::SixHours, &mut state).0.is_empty());
    lib.h.monitor.set(|s| {
        s.power = PowerSource::Ac;
        s.battery_percent = None;
    });
    lib.wait(&auto, "completed");
}

#[test]
fn p4_12_at_launch_a_stale_folder_is_checked_by_the_workspace_itself() {
    let mut lib = Lib::with(fast_config());
    let (root_id, first) = lib.add(&books(&lib, "Books", 3));
    lib.wait(&first, "completed");
    // Closed and opened again a day later.
    lib.ws = lib.env.other_workspace();
    lib.clock.advance(hours(25));
    let mut ws = lib
        .env
        .workspace_with_inventory(lib.h.deps.clone())
        .with_clock(lib.clock.clone());
    ws.startup();
    lib.ws = ws;
    assert!(wait_until(LONG, || lib.scans(&root_id).len() == 2));
    let auto = lib.latest_scan(&root_id);
    lib.wait(&auto, "completed");
    assert_eq!(lib.scans(&root_id)[1].1.as_deref(), Some("automatic"));
}

/// A folder that went away no longer holds up every other folder's scan;
/// its scan carries on when it is back (Slice 4.5).
#[test]
fn a_folder_away_waits_without_blocking_and_resumes_when_back() {
    let lib = Lib::with(fast_config());
    let drive = lib.env.base.join("Drive");
    let away_dir = books(&lib, "Drive/Away", 300);
    lib.h.source.slow(Duration::from_millis(2));
    let (_, away_job) = lib.add(&away_dir);
    assert!(lib.h.source.hold_next_listing(LONG));
    fs::rename(&drive, lib.env.base.join("Unplugged")).unwrap();
    lib.h.source.release();
    lib.wait(&away_job, "paused_policy");

    // Another folder's scan runs and finishes meanwhile.
    lib.h.source.slow(Duration::ZERO);
    let (_, other) = lib.add(&books(&lib, "Other", 3));
    lib.wait(&other, "completed");
    assert_eq!(lib.state(&away_job), "paused_policy");

    // The drive back: the folder check queues the scan again, and it
    // finishes with nothing marked missing.
    fs::rename(lib.env.base.join("Unplugged"), &drive).unwrap();
    lib.wait(&away_job, "completed");
    let ov = control::overview(lib.ws.require_open().unwrap()).unwrap();
    assert_eq!(ov.missing_files, 0);
    let _ = JobState::Completed;
    let _ = lib.clock.now();
}
