/**
 * Words for search (phase1-spec §5.1, §5.2; meaning search, phase2-spec
 * §5.3).
 */
import type { ExtractionOverview } from './api/bindings/ExtractionOverview';
import type { MatchReason } from './api/bindings/MatchReason';
import type { SemanticStatus } from './api/bindings/SemanticStatus';
import type { TitleSource } from './api/bindings/TitleSource';
import { count } from './inventoryCopy';

export const SEARCH_HELP =
  'Searches the text of your books and papers, and their names. Use quotes for an exact phrase. End a word with * to match its beginning.';

export const WHY_LABEL: Record<MatchReason, string> = {
  text: 'In the text',
  name: 'In the name',
  both: 'In the text and the name',
  semantic: 'Similar meaning',
  text_semantic: 'In the text and similar meaning',
};

/** Whether "Some of your words" can apply: never to a meaning match. */
export function foundByWords(why: MatchReason): boolean {
  return why !== 'semantic' && why !== 'text_semantic';
}

/** The one-line note above the results about meaning search, or null. */
export function meaningNote(s: SemanticStatus): string | null {
  switch (s.state) {
    case 'off':
      return null;
    case 'ready':
      return s.coveragePercent < 100
        ? `Meaning search covers ${s.coveragePercent}% of files while the index builds.`
        : null;
    case 'unavailable':
      switch (s.reason) {
        case 'not_permitted':
          return "Showing text matches only: embedding through an internet service isn't allowed in Settings › Privacy.";
        case 'unreachable':
          return "Showing text matches only: the AI service isn't reachable.";
        case 'timeout':
          return 'Showing text matches only: the AI service took too long. Search again in a moment.';
        case 'loading':
          return 'Showing text matches only: the meaning index is still loading.';
        default:
          return "Showing text matches only: meaning search didn't work this time.";
      }
  }
}

/** Whether the shown results may improve by searching again soon. */
export function meaningPending(s: SemanticStatus): boolean {
  return s.state === 'unavailable' && (s.reason === 'loading' || s.reason === 'timeout');
}

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
