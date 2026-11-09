/** Wording for the Library table. */
import type { FileInstance } from './api/bindings/FileInstance';
import type { OsKind } from './api/bindings/OsKind';
import { formatBytes } from './format';

export function statusLabel(item: Pick<FileInstance, 'availability' | 'storage'>): string {
  switch (item.availability) {
    case 'missing':
      return 'Missing';
    case 'unreachable':
      return 'Not connected';
    case 'available':
      return item.storage === 'cloud_only' ? 'Only in the cloud' : 'Available';
  }
}

export function revealLabel(os: OsKind | undefined): string {
  switch (os) {
    case 'macos':
      return 'Show in Finder';
    case 'windows':
      return 'Show in Explorer';
    default:
      return 'Show in folder';
  }
}

export function downloadPrompt(item: { sizeBytes?: number | null }): string {
  const size = item.sizeBytes == null ? '' : ` (${formatBytes(item.sizeBytes)})`;
  return `This file is stored in the cloud. Opening it will download it${size}.`;
}

export function modifiedText(iso: string | null): string {
  if (!iso) return 'Unknown';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return 'Unknown';
  return new Date(t).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
}

export function rangeText(
  page: number,
  pageSize: number,
  shown: number,
  matched: number,
  scanning: boolean,
): string {
  if (matched === 0) return scanning ? 'No matches yet' : 'No matches';
  const from = page * pageSize + 1;
  const to = page * pageSize + shown;
  const noun = matched === 1 ? 'match' : 'matches';
  return `Showing ${from.toLocaleString('en-US')}–${to.toLocaleString('en-US')} of ${matched.toLocaleString('en-US')} ${noun}${scanning ? ' so far' : ''}`;
}
