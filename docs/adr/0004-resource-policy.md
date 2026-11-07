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

**Rates are starting values.** Full rate is 100 metadata operations per
second and slow rate is 20, the spec's engineering starting points. They are
labelled as such in the DTO and the UI, and Slice 0.8 benchmarks will tune
them.

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
