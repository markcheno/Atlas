/**
 * Wording for research trails (Slice 3.3, phase3-spec §5.2, §5.7): the
 * status bar's indicator, New trail, Home's note and Settings › Privacy.
 */
import type { Anchor } from './api/bindings/Anchor';
import type { OpenAction } from './api/bindings/OpenAction';
import type { Reached } from './api/bindings/Reached';
import type { RecordingStatus } from './api/bindings/RecordingStatus';
import type { ResolvedAnchor } from './api/bindings/ResolvedAnchor';
import type { StepFilters } from './api/bindings/StepFilters';
import { count } from './inventoryCopy';

export const TRAIL_RECORDING = 'Trail: recording';
export const TRAIL_PAUSED = 'Trail: paused';
export const TRAIL_UNAVAILABLE = 'Trails unavailable';
export const TRAIL_FAILED = "Atlas couldn't save this step";

/** The indicator's label. */
export function trailLabel(status: RecordingStatus | null): string {
  if (!status || status.state === 'unavailable') return TRAIL_UNAVAILABLE;
  return status.state === 'recording' ? TRAIL_RECORDING : TRAIL_PAUSED;
}

/** What the indicator's button does. */
export function trailToggleLabel(status: RecordingStatus | null): string {
  return status?.state === 'recording' ? 'Pause recording trails' : 'Resume recording trails';
}

export const NEW_TRAIL = 'New trail';
export const NEW_TRAIL_DONE = 'The next search starts a new trail.';

export const FIRST_STEP_NOTE =
  'Atlas now keeps a trail of your searches and questions on this computer, so you can pick them up later. Pause it or delete trails any time.';

export const RESEARCH_TRAILS_TITLE = 'Research trails';
export const RESEARCH_TRAILS_NOTE =
  'Atlas keeps a trail of what you search for, what you ask and what Atlas answered, and the sources you open from them. It stays on this computer, in your library, and is never sent anywhere.';
export const RECORD_LABEL = 'Record my trails';
export const RECORD_PAUSED_NOTE =
  'Paused: nothing is recorded. Searching, asking and saving work as before.';
export const EXPIRE_LABEL = 'Remove unpinned trails after 90 days';
export const EXPIRE_NOTE =
  'Pinned trails, and trails in an interest or a project, are kept. Bookmarks and saved findings never expire.';
export const DELETE_ALL = 'Delete all trails…';

/** The confirmation, naming how many go. */
export function deleteAllQuestion(trails: number): string {
  return trails === 1
    ? 'Delete your 1 trail? Bookmarks and saved findings are kept.'
    : `Delete all ${count(trails, 'trail')}? Bookmarks and saved findings are kept.`;
}

/** After Delete all trails. */
export function deletedTrails(n: number): string {
  return n === 0 ? 'There were no trails to delete.' : `Deleted ${count(n, 'trail')}.`;
}

// ------------------------------------------------ the Trails view (3.4)

export const TRAILS_EMPTY =
  'Atlas keeps a trail of what you search for and ask, on this computer only. Your first search starts one.';
export const PICK_UP_TITLE = 'Pick up where you left off';

export function noTrailsMatch(text: string): string {
  return `No trails match “${text}”`;
}

export function stepsText(n: number): string {
  return count(n, 'step');
}

/** "Nov 11, 2026", or a range when the trail spans days. */
export function trailDates(startedAt: string, lastStepAt: string): string {
  const day = (iso: string) => {
    const t = Date.parse(iso);
    return Number.isNaN(t)
      ? iso
      : new Date(t).toLocaleDateString('en-US', {
          year: 'numeric',
          month: 'short',
          day: 'numeric',
        });
  };
  const [a, b] = [day(startedAt), day(lastStepAt)];
  return a === b ? a : `${a} – ${b}`;
}

/** A step's time: "Nov 11, 2026, 9:05 AM". */
export function stepWhen(at: string): string {
  const t = Date.parse(at);
  return Number.isNaN(t)
    ? at
    : new Date(t).toLocaleString('en-US', {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
        hour: 'numeric',
        minute: '2-digit',
      });
}

const REACHED: Record<Reached, string> = {
  result: 'from a search result',
  citation: 'from a citation',
  related: 'from Related',
  suggestion: 'from a suggestion',
  bookmark: 'from a bookmark',
  trail: 'from a trail',
};

export function reachedLabel(via: Reached): string {
  return REACHED[via];
}

const ACTIONS: Record<OpenAction, string> = {
  details: 'details',
  page: 'the page',
  open: 'opened',
  reveal: 'shown in the file manager',
};

/** "details, then opened". */
export function actionsText(actions: OpenAction[]): string {
  return actions.map((a) => ACTIONS[a]).join(', then ');
}

/** "in Books · PDFs only", or "" for no filters. */
export function filtersText(f: StepFilters): string {
  const parts: string[] = [];
  if (f.folderName) parts.push(`in ${f.folderName}`);
  if (f.fileType) parts.push(f.fileType === 'pdf' ? 'PDFs only' : 'EPUBs only');
  return parts.join(' · ');
}

export function matchedThen(n: number): string {
  return `${count(n, 'document')} matched then`;
}

/** Where an anchor pointed, as it was. */
export function placeText(a: Anchor): string {
  return a.location ?? (a.pageNo ? `Page ${a.pageNo}` : '');
}

export const FILE_GONE = "This file isn't in your library any more.";
export const NOT_READ_YET = "Atlas hasn't read this file yet.";
export const MOVED_WITHIN = 'This passage has moved within the document since.';
export const PASSAGE_NOT_FOUND = "Atlas couldn't find this passage again.";

/** What a resolved anchor says about itself, if anything. */
export function anchorNote(r: ResolvedAnchor): string | null {
  switch (r.state) {
    case 'no_file':
      return FILE_GONE;
    case 'unknown':
      return NOT_READ_YET;
    case 'found_elsewhere':
      return MOVED_WITHIN;
    case 'found':
      return r.excerptFound ? null : PASSAGE_NOT_FOUND;
  }
}

export function citedOn(at: string): string {
  const t = Date.parse(at);
  const day = Number.isNaN(t)
    ? at
    : new Date(t).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: 'numeric' });
  return `Sent to the model for this question on ${day}.`;
}

/** The chip's and the source's name: "Source 2: Title, Chapter 3". */
export function recordedSourceName(n: number, a: Anchor): string {
  const place = placeText(a);
  return place ? `Source ${n}: ${a.title}, ${place}` : `Source ${n}: ${a.title}`;
}

export function folderGone(name: string | null): string {
  return name
    ? `The folder “${name}” isn't in your library any more, so Atlas searched all of it.`
    : "That folder isn't in your library any more, so Atlas searched all of it.";
}

export const CONTINUE = 'Continue this trail';
export const CONTINUING = 'New steps join this trail.';
export const SEARCH_AGAIN = 'Search again';
export const ASK_AGAIN = 'Ask again';
export const ADD_NOTE = 'Add a note';
export const ADD_TRAIL_NOTE = 'Add a note to this trail';
export const NOTE_MAX = 4000;
export const TITLE_MAX = 200;

/** Shown near the limit: "120 characters left". */
export function charactersLeft(text: string, max: number): string | null {
  const left = max - [...text].length;
  return left <= max / 10 ? `${count(Math.max(0, left), 'character')} left` : null;
}

export function deleteTrailQuestion(steps: number): string {
  return `Delete this trail and its ${stepsText(steps)}? Bookmarks and saved findings are kept.`;
}

export const DELETE_STEP_QUESTION = 'Delete this step? Bookmarks and saved findings are kept.';
