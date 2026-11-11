/**
 * Words for Settings › Storage and the duplicates view (phase1-spec §5.5).
 */
import type { IndexingEstimate } from './api/bindings/IndexingEstimate';
import type { KnowledgeStatus } from './api/bindings/KnowledgeStatus';
import type { MeaningEstimate } from './api/bindings/MeaningEstimate';
import { formatBytes } from './format';
import { count } from './inventoryCopy';

/** "1.5–2.5 GB", or "800 MB–1.2 GB" across units. */
export function bytesRange(low: number, high: number): string {
  const [lv, lu] = formatBytes(low).split(' ');
  const [hv, hu] = formatBytes(high).split(' ');
  return lu === hu ? `${lv}–${hv} ${hu}` : `${lv} ${lu}–${hv} ${hu}`;
}

/** The estimate as one sentence; never a single number. */
export function estimateText(e: IndexingEstimate, freeBytes: number | null): string {
  switch (e.kind) {
    case 'all_read':
      return 'Atlas has read the text of every file it can.';
    case 'not_enough_read':
      return 'Not enough read yet to estimate how much space the rest of the text will need.';
    case 'range': {
      const sentence = `Text for the remaining ${count(e.remainingFiles, 'file')} will need about ${bytesRange(e.lowBytes, e.highBytes)}.`;
      return freeBytes !== null && e.highBytes > freeBytes
        ? `${sentence} That may be more than the free space on this drive.`
        : sentence;
    }
  }
}

/** The meaning index's estimate (Slice 2.3); null when nothing waits. */
export function meaningEstimateText(e: MeaningEstimate | null): string | null {
  if (!e || e.remainingFiles === 0) return null;
  return `Meaning for the remaining ${count(e.remainingFiles, 'file')} will need about ${bytesRange(e.lowBytes, e.highBytes)}.`;
}

/** "Snapshots (3)" (Slice 3.2). */
export function snapshotsLabel(n: number): string {
  return `Snapshots (${n})`;
}

/**
 * What Storage says about the knowledge store when something needs saying
 * (Slice 3.2, ADR 0016); null when all is well.
 */
export function knowledgeText(k: KnowledgeStatus | null): string | null {
  if (!k) return null;
  switch (k.state) {
    case 'incompatible':
      return 'Your trails and notes were saved by a newer version of Atlas. Atlas left them exactly as they are: update Atlas to use them.';
    case 'damaged':
      return "Atlas can't read your trails and notes, and left the file exactly as it is. Its snapshots are in the backups/knowledge folder of your library.";
    case 'ready':
      break;
  }
  switch (k.snapshots.lastOutcome) {
    case 'low_space':
      return "This drive is nearly full, so Atlas didn't make a new snapshot of your trails and notes. None was deleted.";
    case 'failed':
      return 'The last snapshot of your trails and notes failed its check and was removed. The earlier ones are kept.';
    default:
      return null;
  }
}

export function freedText(bytes: number): string {
  return bytes > 0 ? `Freed ${formatBytes(bytes)}.` : 'Nothing to clean.';
}

/** "2 groups · 340 KB in extra copies". */
export function duplicatesSummary(groups: number, extraBytes: number): string {
  return `${count(groups, 'group')} · ${formatBytes(extraBytes)} in extra copies`;
}
