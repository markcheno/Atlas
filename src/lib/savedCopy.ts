/** Wording for bookmarks and saved findings (Slice 3.5, phase3-spec §5.3). */
import type { SavedKind } from './api/bindings/SavedKind';
import { count } from './inventoryCopy';

export const BOOKMARK = 'Bookmark';
export const BOOKMARK_PASSAGE = 'Bookmark this passage';
export const BOOKMARKED = 'Bookmarked';
export const SAVE_FINDING = 'Save as a finding';
export const SAVED_FINDING = 'Saved as a finding';

export const SAVED_TAB = 'Saved';
export const TRAILS_TAB = 'Trails';
export const SAVED_EMPTY =
  'Bookmarks and findings you save show here. Bookmark a passage or save an answer to keep it.';
export const BOOKMARKS_HERE = 'Bookmarks in this document';

export const KIND_FILTERS: ReadonlyArray<{ value: SavedKind | ''; label: string }> = [
  { value: '', label: 'All' },
  { value: 'bookmark', label: 'Bookmarks' },
  { value: 'finding', label: 'Findings' },
];

export function noSavedMatch(text: string): string {
  return `Nothing saved matches “${text}”`;
}

export function savedRange(first: number, last: number, total: number): string {
  return `Showing ${first}–${last} of ${count(total, 'item')}`;
}

/** "Saved Nov 11, 2026". */
export function savedOn(at: string): string {
  const t = Date.parse(at);
  return Number.isNaN(t)
    ? 'Saved'
    : `Saved ${new Date(t).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: 'numeric' })}`;
}

export function deleteSavedQuestion(kind: SavedKind): string {
  return kind === 'bookmark'
    ? 'Delete this bookmark? The document stays where it is.'
    : 'Delete this finding? Its trail, if it still has one, is kept.';
}
