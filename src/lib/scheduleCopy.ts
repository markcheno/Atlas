/**
 * Plain-language wording for when background work runs (Slice 4.6,
 * phase4-spec §5.4): the Background work choices, the state they put work
 * in, the reasons the schedule holds a job, and full speed.
 */
import type { BackgroundMode } from './api/bindings/BackgroundMode';
import type { FullSpeedBlock } from './api/bindings/FullSpeedBlock';
import type { PauseReason } from './api/bindings/PauseReason';
import type { ResourceStatus } from './api/bindings/ResourceStatus';
import { resources } from './stores/resources.svelte';

export const MODES: [BackgroundMode, string][] = [
  ['automatic', 'Automatic'],
  ['charging', 'Only while charging'],
  ['hours', 'Only between'],
  ['paused', 'Paused'],
];

export const MODE_NOTE: Record<BackgroundMode, string> = {
  automatic:
    'Atlas reads, indexes and checks folders whenever there is work, slowing on battery and when the computer is busy.',
  charging:
    'Atlas works only while this computer is plugged in. A computer with no battery always counts as plugged in.',
  hours:
    'Atlas works only between these times, by this computer’s clock. The times may cross midnight.',
  paused:
    'Atlas does no background work until you choose otherwise. Searching, asking and opening files still work.',
};

/** Why the schedule holds a job, as the end of a status line. */
export function holdText(reason: PauseReason | null | undefined): string | null {
  switch (reason) {
    case 'paused_by_user':
      return 'Background work is paused in Settings';
    case 'outside_hours': {
      const until = resources.status?.waitingUntil;
      return until ? `Waiting until ${until}` : 'Waiting for the hours set in Settings';
    }
    case 'not_charging':
      return 'Waiting for power';
    default:
      return null;
  }
}

/** What background work is doing under the schedule, for Settings. */
export function stateText(s: ResourceStatus | null): string {
  if (!s) return '';
  if (s.fullSpeed.on) return 'Running at full speed until the work is done';
  if (s.mode === 'paused') return 'Paused';
  if (s.waitingUntil) return `Waiting until ${s.waitingUntil}`;
  if (s.policy.pauseReason === 'not_charging') return 'Waiting for power';
  if (s.policy.mode === 'paused') return 'Paused for now';
  if (s.policy.mode === 'slow') return 'Running slowly';
  return 'Running';
}

/** Why full speed isn't offered now. */
export const BLOCKED: Record<FullSpeedBlock, string> = {
  on_battery: 'Plug in to run at full speed.',
  power_unknown:
    'Atlas can’t tell whether this computer is plugged in, so it won’t run at full speed.',
  paused: 'Background work is paused.',
  held: 'Atlas can’t run background work right now.',
};

export const FULL_SPEED_NOTE =
  'Runs reading, indexing and folder checks without resting, until nothing is waiting, you press Stop, or Atlas quits. Low battery, low space and heat still pause it.';
