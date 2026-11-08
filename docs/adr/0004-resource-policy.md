# ADR 0004: Resource policy for background work

- **Status:** Accepted
- **Date:** 2026-11-07
- **Context documents:** [Phase 0 spec](../phase0-spec.md) sections 5.4, 7.2 and 8;
  [product spec](../product-spec.md) sections 7 and 9

## Context

Indexing a large library can take days, so it must stay low priority. It
must not drain a laptop battery, fill the data drive, or compete with a
system that is under pressure. The OS does not always say what is going on.
Some machines report no battery, some report a battery but not whether it is
charging, and some report no memory or thermal signals at all. The spec
requires two things: an unknown reading must never be treated as a good one,
and a desktop without a battery must never be treated as a laptop on
battery.

## Decision

**One concrete policy, first match wins.** `services/scheduler/policy.rs`
turns a `DeviceSnapshot` into a `PolicyDecision`: run, slow, or paused, with
a machine-readable reason. The rules, in priority order:

1. If the data drive is unreachable, pause.
2. If free space is below `max(2 GiB, min(10 GiB, 2% of capacity))`, pause.
3. If memory pressure or thermal state is critical, pause.
4. If the battery rule applies (below), pause.
5. If the computer is on battery, slow down.
6. If the power source is unknown, slow down.
7. If memory pressure or thermal state is elevated, slow down.
8. Otherwise, on AC or on a desktop, run.

It is a type, not a trait, because Phase 0 has a single job kind.

**Battery hysteresis.** Work pauses on battery at 20% or below, and resumes
only at 25% or when AC returns. The hysteresis state updates on every
sample, even when a higher rule wins. So when low space clears while the
battery is at 22%, work stays paused for the battery.

**Unknown is conservative.** An unknown power source slows work. It never
runs at full rate and never pauses outright, because the readings may simply
be unavailable on that platform. Memory pressure and thermal state that are
unknown impose nothing. On those platforms the bounded rates are the only
protection, and the telemetry gap is reported in the snapshot.

**Desktop is not "on battery".** When the OS reports no batteries,
`batteryPresent` is `false` and power is `ac`. When the battery manager
fails, power is `unknown`. The two cases are never conflated.

**Rates come from measurements.** Full rate is 500 metadata operations per
second and the slow rate is one fifth of that, 100. Phase 0 began at 100 and
20, the spec's engineering starting points. Slice 0.8 replaced them with
measured values (amendment below).

**User pause is separate.** The policy never knows about the user's Pause
button. The 0.5 job state machine keeps `paused_user` and `paused_policy`
apart, so plugging in a cable never resumes work the user paused.

**No watcher thread yet.** In 0.4 snapshots are taken on demand: at startup,
on each `get_device_check`, and every 30 seconds while Settings is open.
The 0.5 scan loop will sample before each batch and call the same
`evaluate`, which also covers the spec's "recheck periodically while
running" requirement. OS wake and power-change notifications are a later
refinement, because polling picks up changes within a batch.

**Debug simulation.** Debug builds honor `ATLAS_SIMULATE_POWER` (`ac`,
`battery:<pct>`, `unknown` or `desktop`) and `ATLAS_SIMULATE_FREE_BYTES`, so
every policy state can be demonstrated on real hardware
(`just dev-simulate battery:18`). Release builds ignore them.

## Consequences

- Policy behavior is fully unit-tested with hand-built snapshots, including
  P0-10, P0-11 and P0-12.
- On Windows, memory pressure and thermal state are unknown, and on Linux
  thermal state is unknown. The bounded rates still apply there.
- Some Macs report a battery state of "unknown" while plugged in and
  holding charge. Atlas reports that truthfully as unknown power and works
  slowly. A platform power-source API could refine this later.
- Thresholds such as 20%, 25%, the rates, and the memory-pressure cutoffs
  can be changed in one place.

## Amendment: measured rates (Slice 0.8, 2026-11-07)

Spec §13 asks for the throttle to be revisited from measurements. The
benchmark harness (`src-tauri/benches/inventory.rs`, release build, real
filesystem and SQLite) cataloged the 30,006-file fixture at fixed rates on
an Apple M5 Pro (18 cores, 64 GB, macOS 27.0.1, on AC):

| Rate (entries/s) | Wall time | Scan CPU, % of one core | Peak RSS |
| ---: | ---: | ---: | ---: |
| unthrottled | 1.3 s | 76.3 | 15 MiB |
| 1,000 | 30 s | 11.1 | 19 MiB |
| 500 | 61 s | 6.8 | 20 MiB |
| 250 | 122 s | 4.5 | 20 MiB |
| 100 (old full rate) | 304 s | 2.4 | 20 MiB |

The rule was the fastest rate that keeps average CPU at or below 25% of one
core, with browsing p95 under 250 ms while scanning. 1,000 passes on this
machine, but only by about 2×, and the minimum supported hardware is
unknown. **500 was chosen**: 3.7× headroom, 30,000 files in about a
minute, and browsing latency unaffected (see
[benchmarks](../benchmarks.md)). Slowed work (battery, unknown power, a
busy system) runs at 100, which is 30,000 files in about 5 minutes.

The CI runners' numbers (shared VMs on all three OSes) are recorded in
[benchmarks](../benchmarks.md) as a check that 500 stays within the rule on
slower machines. Revisit when real users' hardware reports arrive, or when
Phase 1 adds per-file reading, which has a very different cost.
