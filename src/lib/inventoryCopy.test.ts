import { describe, expect, it } from 'vitest';
import { inventoryStatus, overview, rootSummary, scanRecord } from '../test/tauriMock';
import {
  activityText,
  awayText,
  foundText,
  lastCheckLine,
  problemText,
  rootLine,
  scanRecordText,
  statusLine,
} from './inventoryCopy';

const NOW = Date.parse('2026-11-07T12:00:00Z');
const hoursAgo = (h: number) => new Date(NOW - h * 3_600_000).toISOString();

describe('a folder row says what happened (Slice 4.5)', () => {
  const checked = rootSummary({
    fileCount: 1204,
    lastScanAt: hoursAgo(2),
    lastState: 'completed',
  });

  it.each([
    [{ state: 'running', origin: 'automatic' }, 'Checking for changes…'],
    [{ state: 'running', origin: 'user' }, 'Cataloging…'],
    [{ state: 'queued', origin: 'automatic' }, 'Waiting to check'],
    [{ state: 'queued', origin: 'launch' }, 'Waiting to catalog'],
    [{ state: 'paused_user' }, 'Cataloging paused by you'],
    [
      { state: 'paused_policy', pauseReason: 'source_root_missing' },
      'Waiting for the folder to come back',
    ],
    [{ state: 'paused_policy', pauseReason: 'battery_low' }, 'Cataloging paused'],
  ] as const)('while %o: %s', (job, line) => {
    const active = inventoryStatus(job);
    expect(rootLine(checked, overview({ active }), NOW)).toBe(line);
  });

  it('reads another folder’s scan as nothing to do with this one', () => {
    const active = inventoryStatus({ sourceRootId: 'root-other' });
    expect(rootLine(checked, overview({ active }), NOW)).toBe('1,204 files · Checked 2 h ago');
  });

  it('when idle, says when it was checked and what is away', () => {
    const s = rootSummary({ ...checked, cloudOnlyCount: 12, missingCount: 3 });
    expect(rootLine(s, overview(), NOW)).toBe(
      '1,204 files · Checked 2 h ago · 12 only in the cloud · 3 missing',
    );
    expect(rootLine(undefined, overview(), NOW)).toBe('Not cataloged yet');
  });

  it('says how the last check ended', () => {
    const last = (r: Parameters<typeof scanRecord>[0]) =>
      lastCheckLine(rootSummary({ lastScan: scanRecord(r) }));
    expect(last({})).toBe('No changes');
    expect(last({ newFiles: 3, changedFiles: 1, missingFiles: 2 })).toBe(
      '3 new · 1 changed · 2 missing',
    );
    expect(last({ problems: 2 })).toBe("No changes · Couldn't read 2 folders");
    expect(last({ state: 'cancelled' })).toBe('Last check stopped by you');
    expect(last({ state: 'failed_terminal', lastError: 'database' })).toBe(
      "Last check didn't finish · Atlas couldn't save what it found",
    );
    expect(last({ state: 'failed_terminal', lastError: 'other' })).toBe(
      "Last check didn't finish · Try Rescan",
    );
    // A scan from before Slice 4.5 counted nothing, so it claims nothing.
    expect(last({ origin: null })).toBeNull();
    expect(last({ origin: null, problems: 1 })).toBe("Couldn't read 1 folder");
    expect(lastCheckLine(rootSummary())).toBeNull();
  });

  it('explains a folder that is away', () => {
    expect(awayText('unavailable', 'macos')).toBe("Not connected · Atlas checks it when it's back");
    expect(awayText('no_access', 'macos')).toMatch(/System Settings › Privacy & Security/);
    expect(awayText('no_access', 'linux')).toMatch(/permissions/);
    expect(awayText('available', 'macos')).toBeNull();
  });
});

describe('recent checks and problems', () => {
  it('lists each check with who started it, how long, and what it found', () => {
    expect(scanRecordText(scanRecord({ finishedAt: hoursAgo(3), newFiles: 4 }), NOW)).toBe(
      '3 h ago · automatically · 2 s · 4 new',
    );
    expect(
      scanRecordText(scanRecord({ origin: 'user', finishedAt: hoursAgo(1), problems: 1 }), NOW),
    ).toBe("1 h ago · by you · 2 s · No changes · couldn't read 1 folder");
    expect(scanRecordText(scanRecord({ state: 'cancelled', finishedAt: hoursAgo(0.5) }), NOW)).toBe(
      '30 min ago · automatically · stopped by you',
    );
    expect(
      scanRecordText(
        scanRecord({ origin: 'added', state: 'failed_terminal', finishedAt: hoursAgo(5) }),
        NOW,
      ),
    ).toBe("5 h ago · when you added it · didn't finish · something went wrong");
  });

  it('says no changes when nothing changed', () => {
    expect(foundText(scanRecord())).toBe('No changes');
    expect(foundText(scanRecord({ changedFiles: 1_500 }))).toBe('1,500 changed');
  });

  it('gives every problem a reason and a fix', () => {
    for (const code of [
      'permission_denied',
      'cloud_listing_timed_out',
      'not_found',
      'path_undecodable',
      'read_failed',
      'something_new',
    ]) {
      const t = problemText(code, 'macos');
      expect(t.why.length).toBeGreaterThan(0);
      expect(t.fix).toMatch(/Rescan|offline|connected/);
    }
    expect(problemText('permission_denied', 'windows').fix).toMatch(/Properties/);
  });
});

describe('the status bar names Atlas’s own checks', () => {
  it('says checking for changes instead of counting files', () => {
    const active = inventoryStatus({ origin: 'automatic' });
    expect(statusLine(overview({ active }))).toBe('Checking Books for changes');
    expect(statusLine(overview({ active: { ...active, state: 'queued' } }))).toBe(
      'Waiting to check Books',
    );
  });

  it('puts every pipeline’s state in words', () => {
    expect(activityText('catalog', 'running', { origin: 'automatic' })).toBe(
      'Checking for changes',
    );
    expect(activityText('catalog', 'running', { origin: 'user' })).toBe('Cataloging');
    expect(activityText('read', 'queued')).toBe('Waiting to read text');
    expect(activityText('meaning', 'paused_user')).toBe('Indexing meaning paused by you');
    expect(activityText('catalog', 'paused_policy', { pauseReason: 'source_root_missing' })).toBe(
      'Waiting for the folder',
    );
    expect(activityText('read', 'failed_retryable')).toBe('Reading text · retrying');
  });
});
