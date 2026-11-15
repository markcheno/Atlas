//! The busy-computer rule (phase4-spec §7.5, Slice 4.6): when applications
//! other than Atlas keep most of the processor busy, background work slows,
//! and it returns once they ease off.
//!
//! "Other" leaves out Atlas, its workers, and the AI service on this
//! computer that Atlas itself keeps busy while indexing meaning: counting
//! that service would slow Atlas because of its own work. The OS probe is
//! `infrastructure::cpu_probe`; tests use a fake.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Slow when other applications use more than this share of all cores.
/// Provisional, measured in Slice 4.6 (docs/benchmarks.md).
pub const BUSY_ABOVE: f32 = 0.60;
/// Return to normal once they use less than this share.
pub const QUIET_BELOW: f32 = 0.40;
/// The readings are averaged over this long.
pub const WINDOW: Duration = Duration::from_secs(30);
/// One reading at most this often, and only while something asks.
pub const SAMPLE_EVERY: Duration = Duration::from_secs(5);

/// How busy other applications keep the processor.
pub trait CpuProbe: Send {
    /// The share of all cores (0 to 1) other applications used since the
    /// previous call; `None` when it can't be measured (the first call
    /// only starts measuring).
    fn sample(&mut self) -> Option<f32>;
    /// Leaves the processes listening on `ports` out of "other": the AI
    /// service on this computer (an empty list leaves nothing extra out).
    fn exclude_ports(&mut self, _ports: &[u16]) {}
}

/// No readings: the rule never applies (tests, and where nothing can be
/// measured).
#[derive(Debug, Default)]
pub struct NoCpuProbe;

impl CpuProbe for NoCpuProbe {
    fn sample(&mut self) -> Option<f32> {
        None
    }
}

/// The rule's state: recent readings and whether work is slowed.
#[derive(Debug)]
pub struct BusyRule {
    window: Duration,
    every: Duration,
    /// When each reading was taken, how long it covers, and its share.
    readings: VecDeque<(Instant, Duration, f32)>,
    last_sample: Option<Instant>,
    busy: bool,
}

impl Default for BusyRule {
    fn default() -> Self {
        Self::new(WINDOW, SAMPLE_EVERY)
    }
}

impl BusyRule {
    /// A rule averaging over `window`, reading at most every `every`
    /// (tests shorten both).
    pub fn new(window: Duration, every: Duration) -> Self {
        Self {
            window,
            every,
            readings: VecDeque::new(),
            last_sample: None,
            busy: false,
        }
    }

    /// Reads `probe` if a reading is due, and says whether other
    /// applications keep the computer busy. The rule decides only once its
    /// readings cover most of the window; until then, and when nothing can
    /// be measured, it keeps its last answer, starting from "not busy".
    pub fn update(&mut self, now: Instant, probe: &mut dyn CpuProbe) -> bool {
        let due = self
            .last_sample
            .is_none_or(|t| now.saturating_duration_since(t) >= self.every);
        if due {
            let covered = self
                .last_sample
                .map(|t| now.saturating_duration_since(t))
                .unwrap_or_default();
            self.last_sample = Some(now);
            match probe.sample() {
                Some(share) if !covered.is_zero() => {
                    self.readings
                        .push_back((now, covered, share.clamp(0.0, 1.0)));
                }
                Some(_) => {}
                // Nothing measurable: the rule doesn't apply.
                None => {
                    self.readings.clear();
                    self.busy = false;
                }
            }
        }
        while self
            .readings
            .front()
            .is_some_and(|(at, _, _)| now.saturating_duration_since(*at) > self.window)
        {
            self.readings.pop_front();
        }
        let covered: Duration = self.readings.iter().map(|(_, d, _)| *d).sum();
        if covered >= self.window.mul_f32(0.8) {
            let weighted: f32 = self
                .readings
                .iter()
                .map(|(_, d, s)| d.as_secs_f32() * s)
                .sum();
            let average = weighted / covered.as_secs_f32();
            if average > BUSY_ABOVE {
                self.busy = true;
            } else if average < QUIET_BELOW {
                self.busy = false;
            }
        }
        self.busy
    }

    pub fn busy(&self) -> bool {
        self.busy
    }
}

/// One process in a reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub parent: Option<u32>,
    /// Running Atlas's own executable (its workers do).
    pub atlas_exe: bool,
    /// Percent of one core since the previous reading (may exceed 100).
    pub cpu_percent: f32,
}

/// The share of all `cores` used by processes other than Atlas (`own`),
/// its workers (children running Atlas's executable), and the `excluded`
/// processes (the AI service) with their descendants.
pub fn other_share(procs: &[Proc], own: u32, excluded: &[u32], cores: usize) -> f32 {
    if cores == 0 {
        return 0.0;
    }
    let parents: HashMap<u32, Option<u32>> = procs.iter().map(|p| (p.pid, p.parent)).collect();
    let under_excluded = |pid: u32| {
        let mut at = Some(pid);
        // Bounded: a reading taken while processes come and go may hold a
        // cycle of reused process ids.
        for _ in 0..64 {
            let Some(p) = at else { return false };
            if excluded.contains(&p) {
                return true;
            }
            at = parents.get(&p).copied().flatten();
        }
        false
    };
    let used: f32 = procs
        .iter()
        .filter(|p| p.pid != own)
        .filter(|p| !(p.atlas_exe && p.parent == Some(own)))
        .filter(|p| !under_excluded(p.pid))
        .map(|p| p.cpu_percent.max(0.0))
        .sum();
    (used / (cores as f32 * 100.0)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(VecDeque<Option<f32>>);

    impl CpuProbe for Fake {
        fn sample(&mut self) -> Option<f32> {
            self.0.pop_front().flatten()
        }
    }

    fn fake(readings: &[Option<f32>]) -> Fake {
        Fake(readings.iter().copied().collect())
    }

    const S: Duration = Duration::from_secs(1);

    /// Feeds one reading every 5 s; returns the rule's answers.
    fn run(rule: &mut BusyRule, probe: &mut Fake, start: Instant, n: u32) -> Vec<bool> {
        (0..n)
            .map(|i| rule.update(start + 5 * S * i, probe))
            .collect()
    }

    #[test]
    fn slows_above_sixty_percent_and_returns_below_forty() {
        let start = Instant::now();
        let mut rule = BusyRule::default();
        // The first reading covers no time; then 70% every 5 s.
        let mut probe = fake(&[
            Some(0.0),
            Some(0.7),
            Some(0.7),
            Some(0.7),
            Some(0.7),
            Some(0.7),
        ]);
        probe.0.push_back(Some(0.7));
        let answers = run(&mut rule, &mut probe, start, 7);
        // Busy only once the readings cover most of the window (24 s).
        assert_eq!(answers, [false, false, false, false, false, true, true]);

        // Between the thresholds it stays busy; under 40% it returns.
        let later = start + 35 * S;
        let mut probe = fake(&[Some(0.5); 7]);
        assert!(run(&mut rule, &mut probe, later, 7).iter().all(|b| *b));
        let mut probe = fake(&[Some(0.1); 8]);
        let answers = run(&mut rule, &mut probe, later + 35 * S, 8);
        assert!(answers[0], "one quiet reading doesn't end it");
        assert!(!answers[7], "a quiet half-minute does");
    }

    #[test]
    fn readings_are_taken_at_most_every_five_seconds() {
        let start = Instant::now();
        let mut rule = BusyRule::default();
        let mut probe = fake(&[Some(0.0), Some(0.9)]);
        rule.update(start, &mut probe);
        rule.update(start + S, &mut probe);
        rule.update(start + 2 * S, &mut probe);
        assert_eq!(probe.0.len(), 1, "two calls within 5 s took no reading");
    }

    #[test]
    fn a_missing_reading_means_the_rule_does_not_apply() {
        let start = Instant::now();
        let mut rule = BusyRule::new(10 * S, S);
        let mut probe = fake(&[Some(0.0); 12]);
        probe.0.iter_mut().skip(1).for_each(|r| *r = Some(0.95));
        for i in 0..12 {
            rule.update(start + S * i, &mut probe);
        }
        assert!(rule.busy());
        let mut none = NoCpuProbe;
        assert!(!rule.update(start + 13 * S, &mut none));
    }

    #[test]
    fn other_share_leaves_out_atlas_its_workers_and_the_ai_service() {
        let p = |pid, parent, atlas_exe, cpu_percent| Proc {
            pid,
            parent,
            atlas_exe,
            cpu_percent,
        };
        let procs = [
            p(1, None, false, 10.0),         // launchd: counts
            p(100, Some(1), true, 400.0),    // Atlas
            p(101, Some(100), true, 300.0),  // its reading worker
            p(102, Some(100), false, 50.0),  // a child that isn't a worker: counts
            p(200, Some(1), false, 600.0),   // the AI service
            p(201, Some(200), false, 200.0), // its helper
            p(300, Some(1), false, 140.0),   // another app: counts
        ];
        let share = other_share(&procs, 100, &[200], 4);
        assert!((share - 0.5).abs() < 1e-6, "{share}");
        // With no service excluded, it counts.
        let share = other_share(&procs, 100, &[], 10);
        assert!((share - 1.0).abs() < 1e-6, "{share}");
        assert_eq!(other_share(&procs, 100, &[], 0), 0.0);
        // A cycle of process ids can't loop forever.
        let cycle = [p(5, Some(6), false, 10.0), p(6, Some(5), false, 10.0)];
        assert!((other_share(&cycle, 100, &[7], 1) - 0.2).abs() < 1e-6);
    }
}
