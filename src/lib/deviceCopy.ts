/**
 * Plain-language wording for device readings, the assessment, and the
 * resource policy. Unknown readings are always shown as "Unknown".
 */
import type { AssessmentNote } from './api/bindings/AssessmentNote';
import type { DeviceAssessment } from './api/bindings/DeviceAssessment';
import type { DeviceSnapshot } from './api/bindings/DeviceSnapshot';
import type { PolicyDecision } from './api/bindings/PolicyDecision';
import { formatBytes } from './format';

export const PROVISIONAL_NOTE =
  'These are early estimates, not tested requirements. Atlas never refuses to run because of them.';

/** Installed memory is conventionally reported in binary gigabytes. */
export function memoryText(bytes: number | null): string {
  if (bytes === null) return 'Unknown';
  return `${Math.round(bytes / 2 ** 30)} GB`;
}

export function coresText(cores: number | null): string {
  return cores === null ? 'Unknown' : `${cores} ${cores === 1 ? 'core' : 'cores'}`;
}

export function dataDriveText(s: DeviceSnapshot): string {
  if (s.dataVolumeReachable === null) return 'No library open';
  if (!s.dataVolumeReachable) return 'Not connected';
  if (s.dataVolumeFreeBytes === null) return 'Unknown';
  return `${formatBytes(s.dataVolumeFreeBytes)} free of ${formatBytes(s.dataVolumeTotalBytes)}`;
}

function pct(s: DeviceSnapshot): string {
  return s.batteryPercent === null ? '' : ` · ${Math.round(s.batteryPercent)}%`;
}

export function powerText(s: DeviceSnapshot): string {
  if (s.batteryPresent === false) return 'No battery (desktop)';
  switch (s.power) {
    case 'battery':
      return `On battery${pct(s)}`;
    case 'ac':
      return s.charging ? `Charging${pct(s)}` : `Plugged in${pct(s)}`;
    case 'unknown':
      return 'Unknown';
  }
}

export function recommendation(a: DeviceAssessment): string {
  if (a.library === 'unknown') {
    return "Atlas couldn't read this computer's memory, so it will start conservatively. Browsing and search will still work.";
  }
  if (a.library === 'usable') {
    return 'Atlas will work well for browsing and searching, though very large libraries may feel slower. For AI features, connecting a service will work better than running models on this computer.';
  }
  switch (a.localAi) {
    case 'likely':
      return 'This computer can comfortably catalog and search your books, and it can likely run AI models locally if you want to.';
    case 'possible':
      return 'This computer can comfortably catalog and search your books. Local AI may be slower; you can connect a service instead.';
    default:
      return 'This computer can comfortably catalog and search your books. For AI features, connecting a service will work better than running models on this computer.';
  }
}

export const NOTE_TEXT: Record<AssessmentNote, string> = {
  on_battery:
    "You're on battery. Atlas will catalog slowly to save power, and pauses if the battery drops to 20%.",
  power_unknown:
    "Atlas can't tell whether this computer is plugged in, so it will work slowly to be safe.",
  desktop: 'No battery detected, so Atlas treats this computer as plugged in.',
  low_space:
    "The drive holding Atlas's data is nearly full. Atlas will pause new work until space is freed.",
  memory_pressure: 'This computer is short on memory right now, so Atlas will slow down.',
  unknown_memory: "Atlas couldn't read how much memory this computer has.",
};

/** Completes "When Atlas catalogs your folders, it will …". */
export function policySentence(p: PolicyDecision, s: DeviceSnapshot): string {
  if (p.mode === 'run') return 'run normally.';
  if (p.mode === 'slow') {
    switch (p.throttleReason) {
      case 'on_battery':
        return 'run slowly because this computer is on battery.';
      case 'power_unknown':
        return "run slowly because Atlas can't tell whether this computer is plugged in.";
      default:
        return 'run slowly because the computer is busy.';
    }
  }
  switch (p.pauseReason) {
    case 'battery_low':
      return `pause because the battery is at ${Math.round(s.batteryPercent ?? 0)}%. It resumes at 25% or when you plug in.`;
    case 'low_disk_space':
      return `pause because the drive holding Atlas's data is nearly full. It resumes when at least ${formatBytes(s.requiredFreeBytes)} is free.`;
    case 'data_root_missing':
      return "pause because the drive holding Atlas's data isn't connected.";
    case 'memory_pressure':
      return 'pause because the computer is short on memory.';
    case 'thermal_pressure':
      return 'pause because the computer is running hot.';
    default:
      return 'pause for now.';
  }
}
