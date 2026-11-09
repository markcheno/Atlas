/**
 * Words for text-reading status (phase1-spec §5.3, §5.4). Reason codes
 * come from Rust; the display text lives here. While cataloging is still
 * finding files, counts are "so far", never "of N" (phase0-spec §5.6).
 */
import type { ExtractionCounts } from './api/bindings/ExtractionCounts';
import type { ExtractionOverview } from './api/bindings/ExtractionOverview';
import type { ExtractionState } from './api/bindings/ExtractionState';
import type { FileInstance } from './api/bindings/FileInstance';
import type { PauseReason } from './api/bindings/PauseReason';
import { count } from './inventoryCopy';

export const TEXT_LABEL: Record<ExtractionState, string> = {
  done: 'Text ready',
  pending: 'Waiting',
  reading: 'Reading…',
  needs_ocr: 'Needs OCR',
  encrypted: 'Encrypted',
  cloud_only: 'Only in the cloud',
  unreadable: "Couldn't read",
};

/** A symbol beside each label, so the badge doesn't rely on colour. */
export const TEXT_ICON: Record<ExtractionState, string> = {
  done: '✓',
  pending: '…',
  reading: '↻',
  needs_ocr: '⊘',
  encrypted: '🔒',
  cloud_only: '☁',
  unreadable: '!',
};

/** The filter's choices, in order. */
export const TEXT_FILTERS: { value: ExtractionState; label: string }[] = [
  { value: 'done', label: 'Text ready' },
  { value: 'pending', label: 'Waiting' },
  { value: 'needs_ocr', label: 'Needs OCR' },
  { value: 'encrypted', label: 'Encrypted' },
  { value: 'cloud_only', label: 'Only in the cloud' },
  { value: 'unreadable', label: "Couldn't read" },
];

const REASONS: Record<string, string> = {
  no_text_layer:
    "This PDF has no readable text layer. It looks scanned; Atlas can't read it without OCR.",
  unreadable_text:
    "This file's text uses fonts Atlas can't decode into letters; it needs OCR to be read.",
  password: 'This PDF needs a password to open.',
  security_handler: 'This PDF is protected and can’t be opened.',
  drm: 'This book is protected by DRM, so its text can’t be read.',
  not_a_pdf: "This file isn't really a PDF.",
  not_an_epub: "This file isn't really an EPUB.",
  zip_slip: 'This book contains unsafe file names, so Atlas didn’t open it.',
  entry_too_large: 'Part of this book is too large to read safely.',
  archive_too_large: 'This book is too large to read safely.',
  max_chars: 'This document has more text than Atlas reads from one file.',
  worker_crashed: 'Reading this file failed twice, so Atlas stopped trying.',
  permission_denied: "Atlas isn't allowed to read this file.",
  read_failed: "Atlas couldn't read this file from the disk.",
  passage_truncated: 'Some very long pages were shortened.',
  not_a_zip: 'This EPUB is damaged: it isn’t a readable archive.',
  epub_structure: 'This EPUB is damaged: its table of contents can’t be read.',
  chapter_unreadable: 'This EPUB is damaged: a chapter can’t be read.',
  chapter_html: 'This EPUB is damaged: a chapter can’t be read.',
  zip_entry: 'This EPUB is damaged.',
  pdf_format: 'This PDF is damaged or incomplete.',
  pdf_error: 'This PDF is damaged or incomplete.',
  open_failed: "Atlas couldn't open this file.",
  unreadable: "Atlas isn't allowed to read this file.",
  cloud_only: 'Its contents are only in the cloud.',
};

/** The effective state for a row: `reading` for the file being read now. */
export function rowState(item: FileInstance, ov: ExtractionOverview | null): ExtractionState {
  const reading = ov?.active?.state === 'running' && ov.active.currentFileId === item.id;
  return reading ? 'reading' : item.extraction.state;
}

/** Plain words for why a file has no text, if any. */
export function reasonText(item: FileInstance): string | null {
  const { state, reason } = item.extraction;
  if (state === 'cloud_only') {
    return 'Its contents are only in the cloud. Atlas reads it once it is downloaded.';
  }
  if (!reason) return null;
  return REASONS[reason] ?? "Atlas couldn't read this file's text.";
}

function pausedText(reason: PauseReason | null, folder: string): string {
  switch (reason) {
    case 'battery_low':
      return 'Paused reading text · Resumes at 25% or when plugged in';
    case 'low_disk_space':
      return 'Paused reading text · Not enough free space';
    case 'data_root_missing':
      return "Paused reading text · The library location isn't available";
    case 'source_root_missing':
      return `Paused reading text · “${folder}” isn't available`;
    case 'memory_pressure':
      return 'Paused reading text · The computer is low on memory';
    case 'thermal_pressure':
      return 'Paused reading text · The computer is too hot';
    default:
      return 'Paused reading text';
  }
}

const readable = (c: ExtractionCounts) => c.done;

/** The status bar's second line (phase1-spec §5.4). */
export function extractionLine(ov: ExtractionOverview | null): string | null {
  if (!ov || ov.counts.total === 0) return null;
  const a = ov.active;
  const folder = a?.sourceRootDisplayPath.split(/[\\/]/).filter(Boolean).pop() ?? '';
  const processed = ov.counts.total - ov.counts.pending;
  const progress = ov.scanning
    ? `${count(processed, 'file')} so far`
    : `${processed.toLocaleString('en-US')} of ${count(ov.counts.total, 'file')}`;
  if (!a) {
    if (ov.counts.pending > 0) return `Text waiting · ${progress} read`;
    if (ov.scanning) return `Text read so far · ${count(processed, 'file')}`;
    return `Text up to date · ${readable(ov.counts).toLocaleString('en-US')} of ${count(ov.counts.total, 'file')} readable`;
  }
  switch (a.state) {
    case 'running':
      if (a.throttleReason === 'on_battery')
        return `Reading text · ${progress} · Slowed on battery`;
      if (a.throttleReason) return `Reading text · ${progress} · Going slowly`;
      return `Reading text · ${progress}`;
    case 'queued':
      return `Waiting to read text · ${progress}`;
    case 'paused_policy':
      return pausedText(a.pauseReason, folder);
    case 'paused_user':
      return `Paused reading text by you · ${progress}`;
    case 'failed_retryable':
      return 'Having trouble reading text · Retrying';
    default:
      return `Reading text · ${progress}`;
  }
}

/** A short text summary for a folder line. */
export function rootTextLine(ov: ExtractionOverview | null, sourceRootId: string): string | null {
  const root = ov?.roots.find((r) => r.sourceRootId === sourceRootId);
  if (!root || root.counts.total === 0) return null;
  if (root.activeJobId) {
    const job = ov?.active?.jobId === root.activeJobId ? ov.active : null;
    return job?.state.startsWith('paused') ? 'Reading text paused' : 'Reading text…';
  }
  const c = root.counts;
  const parts = [`Text: ${c.done.toLocaleString('en-US')} ready`];
  if (c.pending) parts.push(`${c.pending.toLocaleString('en-US')} waiting`);
  const blocked = c.needsOcr + c.encrypted + c.cloudOnly + c.unreadable;
  if (blocked) parts.push(`${blocked.toLocaleString('en-US')} without text`);
  return parts.join(' · ');
}
