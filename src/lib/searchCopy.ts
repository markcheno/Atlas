/**
 * Words for full-text search (phase1-spec §5.1, §5.2).
 */
import type { ExtractionOverview } from './api/bindings/ExtractionOverview';
import type { MatchReason } from './api/bindings/MatchReason';
import type { TitleSource } from './api/bindings/TitleSource';
import { count } from './inventoryCopy';

export const SEARCH_HELP =
  'Searches the text of your books and papers, and their names. Use quotes for an exact phrase. End a word with * to match its beginning.';

export const WHY_LABEL: Record<MatchReason, string> = {
  text: 'In the text',
  name: 'In the name',
  both: 'In the text and the name',
};

/** Where a title came from, when it isn't the file's own metadata. */
export const TITLE_SOURCE_NOTE: Record<TitleSource, string | null> = {
  metadata: null,
  contents: 'Title from the first page',
  file_name: 'Title from the file name',
};

/** False for input with no letters or digits: nothing to search for. */
export function hasWords(text: string): boolean {
  return /[\p{L}\p{N}]/u.test(text);
}

/** The one-line note while text reading is still going on. */
export function readingNote(read: number, total: number): string {
  return `Atlas is still reading text: ${read.toLocaleString('en-US')} of ${count(total, 'file')} so far. Search covers what's been read.`;
}

/** Home's note, from the status bar's numbers; null when nothing is left to read. */
export function homeReadingNote(ov: ExtractionOverview | null): string | null {
  if (!ov?.active || ov.counts.pending === 0) return null;
  return readingNote(ov.counts.total - ov.counts.pending, ov.counts.total);
}

export function readSoFar(read: number, total: number): string {
  return `Atlas has read the text of ${read.toLocaleString('en-US')} of ${count(total, 'file')}.`;
}

/** "Showing 1–20 of 57 documents", "… of 500 or more documents". */
export function resultRange(
  page: number,
  pageSize: number,
  shown: number,
  matched: number,
  capped: boolean,
): string {
  const first = page * pageSize + 1;
  const last = page * pageSize + shown;
  const of = capped
    ? `${matched.toLocaleString('en-US')} or more documents`
    : count(matched, 'document');
  return `Showing ${first.toLocaleString('en-US')}–${last.toLocaleString('en-US')} of ${of}`;
}

export function alsoIn(n: number): string | null {
  if (n <= 0) return null;
  return n === 1 ? 'Also in 1 other place' : `Also in ${n.toLocaleString('en-US')} other places`;
}
