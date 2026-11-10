/**
 * Words for Settings › Storage and the duplicates view (phase1-spec §5.5).
 */
import type { IndexingEstimate } from './api/bindings/IndexingEstimate';
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

export function freedText(bytes: number): string {
  return bytes > 0 ? `Freed ${formatBytes(bytes)}.` : 'Nothing to clean.';
}

/** "2 groups · 340 KB in extra copies". */
export function duplicatesSummary(groups: number, extraBytes: number): string {
  return `${count(groups, 'group')} · ${formatBytes(extraBytes)} in extra copies`;
}
