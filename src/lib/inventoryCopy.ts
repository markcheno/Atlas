/**
 * Plain-language cataloging status. Counts only: Atlas never shows a total
 * or a percentage before it has finished looking (phase0-spec §5.6).
 */
import type { InventoryOverview } from './api/bindings/InventoryOverview';
import type { InventoryStatus } from './api/bindings/InventoryStatus';
import type { JobState } from './api/bindings/JobState';
import type { OsKind } from './api/bindings/OsKind';
import type { RootInventorySummary } from './api/bindings/RootInventorySummary';
import type { ScanOrigin } from './api/bindings/ScanOrigin';
import type { ScanRecord } from './api/bindings/ScanRecord';
import type { SourceRootStatus } from './api/bindings/SourceRootStatus';
import type { ThrottleReason } from './api/bindings/ThrottleReason';
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

/**
 * Why background work is going slowly, for the end of a status line
 * (Slice 4.1.3: the line said "Going slowly" without saying why, and the
 * cataloging line said nothing when the computer was busy).
 */
export function slowedText(reason: ThrottleReason | null | undefined): string {
  switch (reason) {
    case 'on_battery':
      return 'Slowed on battery';
    case 'power_unknown':
      return 'Going slowly · Can’t tell if it’s plugged in';
    case 'system_busy':
      return 'Going slowly · The computer is busy';
    default:
      return '';
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
  const automatic = a.origin === 'automatic';
  switch (a.state) {
    case 'running': {
      const slowed = a.throttleReason ? ` · ${slowedText(a.throttleReason)}` : '';
      // Atlas's own check says less: "a check that finds nothing says
      // nothing" (phase4-spec §5.3).
      if (automatic) return `Checking ${name} for changes${slowed}`;
      return `Cataloged ${count(a.discoveredFiles, 'file')} · Scanning ${name}${slowed}`;
    }
    case 'queued':
      return automatic ? `Waiting to check ${name}` : `Waiting to catalog ${name}`;
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

/** One line per library folder: what it holds and when it was checked. */
export function rootLine(
  summary: RootInventorySummary | undefined,
  ov: InventoryOverview | null,
  now = Date.now(),
): string {
  if (!summary) return 'Not cataloged yet';
  const active = ov?.activeJobs.find((j) => j.sourceRootId === summary.sourceRootId) ?? null;
  const automatic = active?.origin === 'automatic';
  if (active?.state === 'running') return automatic ? 'Checking for changes…' : 'Cataloging…';
  if (active?.state === 'queued' || (!active && summary.lastState === 'queued'))
    return automatic ? 'Waiting to check' : 'Waiting to catalog';
  if (active?.state === 'paused_user') return 'Cataloging paused by you';
  if (active?.state === 'paused_policy')
    return active.pauseReason === 'source_root_missing'
      ? 'Waiting for the folder to come back'
      : 'Cataloging paused';
  if (!summary.lastScanAt) return summary.activeJobId ? 'Waiting to catalog' : 'Not cataloged yet';
  const parts = [
    count(summary.fileCount, 'file'),
    `Checked ${relativeTime(summary.lastScanAt, now)}`,
  ];
  if (summary.cloudOnlyCount > 0)
    parts.push(`${summary.cloudOnlyCount.toLocaleString('en-US')} only in the cloud`);
  if (summary.missingCount > 0)
    parts.push(`${summary.missingCount.toLocaleString('en-US')} missing`);
  return parts.join(' · ');
}

/** What a scan found: "No changes", "3 new · 1 changed · 2 missing". */
export function foundText(r: ScanRecord): string {
  const parts: string[] = [];
  if (r.newFiles) parts.push(`${r.newFiles.toLocaleString('en-US')} new`);
  if (r.changedFiles) parts.push(`${r.changedFiles.toLocaleString('en-US')} changed`);
  if (r.missingFiles) parts.push(`${r.missingFiles.toLocaleString('en-US')} missing`);
  return parts.length ? parts.join(' · ') : 'No changes';
}

const FAILED: Record<string, string> = {
  path_undecodable: "Atlas can't read this folder's name",
  nested_root: "It's inside another library folder",
  database: "Atlas couldn't save what it found",
};

/**
 * How the folder's last check ended, or null when there is nothing to
 * add (a scan from before Slice 4.5 counted nothing).
 */
export function lastCheckLine(summary: RootInventorySummary | undefined): string | null {
  const r = summary?.lastScan;
  if (!r) return null;
  if (r.state === 'cancelled') return 'Last check stopped by you';
  if (r.state === 'failed_terminal')
    return `Last check didn't finish · ${FAILED[r.lastError ?? ''] ?? 'Try Rescan'}`;
  const parts = r.origin === null ? [] : [foundText(r)];
  if (r.problems) parts.push(`Couldn't read ${count(r.problems, 'folder')}`);
  return parts.length ? parts.join(' · ') : null;
}

const ORIGIN: Record<ScanOrigin, string> = {
  added: 'when you added it',
  launch: 'when Atlas opened',
  user: 'by you',
  automatic: 'automatically',
};

/** One line of a folder's recent checks. */
export function scanRecordText(r: ScanRecord, now = Date.now()): string {
  const at = r.finishedAt ?? r.startedAt;
  const parts = [at ? relativeTime(at, now) : 'Earlier'];
  if (r.origin) parts.push(ORIGIN[r.origin]);
  if (r.state === 'completed') {
    parts.push(elapsedText(r.elapsedMs));
    if (r.origin) parts.push(foundText(r));
    if (r.problems) parts.push(`couldn't read ${count(r.problems, 'folder')}`);
  } else if (r.state === 'cancelled') {
    parts.push('stopped by you');
  } else {
    parts.push(`didn't finish · ${FAILED[r.lastError ?? ''] ?? 'something went wrong'}`);
  }
  return parts.join(' · ');
}

/** Why Atlas couldn't read a folder, and what to do about it. */
export function problemText(code: string, os: OsKind | undefined): { why: string; fix: string } {
  switch (code) {
    case 'permission_denied':
      return {
        why: "Atlas isn't allowed to read it",
        fix:
          os === 'macos'
            ? 'Give Atlas access in System Settings › Privacy & Security › Files and Folders, then Rescan.'
            : os === 'windows'
              ? "Check the folder's Security settings in its Properties, then Rescan."
              : "Check the folder's permissions, then Rescan.",
      };
    case 'cloud_listing_timed_out':
      return {
        why: "The cloud service didn't list it in time",
        fix: 'Make the folder available offline, or Rescan later.',
      };
    case 'not_found':
      return {
        why: 'It was moved or deleted while Atlas was looking',
        fix: 'Rescan to check the folder again.',
      };
    case 'path_undecodable':
      return {
        why: "Its name can't be read on this computer",
        fix: 'Rename the folder, then Rescan.',
      };
    default:
      return {
        why: "Atlas couldn't list it",
        fix: 'Check that the drive is connected, then Rescan.',
      };
  }
}

/** For a folder that isn't reachable: what it means and what happens. */
export function awayText(status: SourceRootStatus, os: OsKind | undefined): string | null {
  if (status === 'unavailable') return "Not connected · Atlas checks it when it's back";
  if (status === 'no_access')
    return os === 'macos'
      ? 'Give Atlas access in System Settings › Privacy & Security › Files and Folders'
      : "Check the folder's permissions so Atlas can read it";
  return null;
}

type Pipeline = 'catalog' | 'read' | 'meaning';
const DOING: Record<Pipeline, [string, string, string]> = {
  // running, waiting, paused
  catalog: ['Cataloging', 'Waiting to catalog', 'Cataloging paused'],
  read: ['Reading text', 'Waiting to read text', 'Reading paused'],
  meaning: ['Indexing meaning', 'Waiting to index meaning', 'Indexing meaning paused'],
};

/**
 * One pipeline's state for a folder, in words, for the status bar's list
 * of every folder's activity (Slice 4.5).
 */
export function activityText(
  pipeline: Pipeline,
  state: JobState,
  extra: { origin?: ScanOrigin | null; pauseReason?: string | null } = {},
): string {
  const [running, waiting, paused] = DOING[pipeline];
  const automatic = pipeline === 'catalog' && extra.origin === 'automatic';
  switch (state) {
    case 'running':
      return automatic ? 'Checking for changes' : running;
    case 'queued':
      return automatic ? 'Waiting to check' : waiting;
    case 'paused_user':
      return `${paused} by you`;
    case 'paused_policy':
      return extra.pauseReason === 'source_root_missing' ? 'Waiting for the folder' : paused;
    case 'failed_retryable':
      return `${running} · retrying`;
    default:
      return running;
  }
}
