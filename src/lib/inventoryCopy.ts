/**
 * Plain-language cataloging status. Counts only: Atlas never shows a total
 * or a percentage before it has finished looking (phase0-spec §5.6).
 */
import type { InventoryOverview } from './api/bindings/InventoryOverview';
import type { InventoryStatus } from './api/bindings/InventoryStatus';
import type { RootInventorySummary } from './api/bindings/RootInventorySummary';
import { folderName } from './stores/sources.svelte';

export function count(n: number, one: string, many = `${one}s`): string {
  return `${n.toLocaleString('en-US')} ${n === 1 ? one : many}`;
}

function pausedText(a: InventoryStatus, name: string): string {
  switch (a.pauseReason) {
    case 'battery_low':
      return 'Paused on battery · Resumes at 25% or when plugged in';
    case 'low_disk_space':
      return "Paused · The drive with Atlas's data is nearly full";
    case 'data_root_missing':
      return "Paused · The drive with Atlas's data isn't connected";
    case 'source_root_missing':
      return `Paused · “${name}” isn't available`;
    case 'memory_pressure':
      return 'Paused · The computer is short on memory';
    case 'thermal_pressure':
      return 'Paused · The computer is running hot';
    default:
      return 'Paused';
  }
}

export function statusLine(ov: InventoryOverview | null): string {
  if (!ov) return 'Not cataloging';
  const a = ov.active;
  if (!a) {
    if (ov.roots.length === 0) return 'No library folders yet';
    return `Up to date · ${count(ov.catalogedFiles, 'file')}`;
  }
  const name = folderName(a.sourceRootDisplayPath);
  switch (a.state) {
    case 'running': {
      const slowed =
        a.throttleReason === 'on_battery'
          ? ' · Slowed on battery'
          : a.throttleReason === 'power_unknown'
            ? ' · Going slowly'
            : '';
      return `Cataloged ${count(a.discoveredFiles, 'file')} · Scanning ${name}${slowed}`;
    }
    case 'queued':
      return `Waiting to catalog ${name}`;
    case 'paused_user':
      return `Paused by you · ${count(a.discoveredFiles, 'file')} so far`;
    case 'paused_policy':
      return pausedText(a, name);
    case 'failed_retryable':
      return 'Having trouble cataloging · Retrying';
    default:
      return `Up to date · ${count(ov.catalogedFiles, 'file')}`;
  }
}

export function elapsedText(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min ${s % 60} s`;
  return `${Math.floor(m / 60)} h ${m % 60} min`;
}

export function relativeTime(iso: string, now = Date.now()): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return iso;
  const minutes = Math.round((now - then) / 60_000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  return new Date(then).toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}

/** One line per library folder. */
export function rootLine(
  summary: RootInventorySummary | undefined,
  ov: InventoryOverview | null,
): string {
  if (!summary) return 'Not cataloged yet';
  const active = ov?.active?.sourceRootId === summary.sourceRootId ? ov.active : null;
  if (active?.state === 'running') return 'Cataloging…';
  if (active?.state === 'queued' || summary.lastState === 'queued') return 'Waiting to catalog';
  if (active && (active.state === 'paused_user' || active.state === 'paused_policy')) {
    return 'Cataloging paused';
  }
  if (!summary.lastScanAt) return summary.activeJobId ? 'Waiting to catalog' : 'Not cataloged yet';
  const parts = [count(summary.fileCount, 'file'), `Updated ${relativeTime(summary.lastScanAt)}`];
  if (summary.cloudOnlyCount > 0)
    parts.push(`${summary.cloudOnlyCount.toLocaleString('en-US')} only in the cloud`);
  if (summary.missingCount > 0)
    parts.push(`${summary.missingCount.toLocaleString('en-US')} missing`);
  return parts.join(' · ');
}
