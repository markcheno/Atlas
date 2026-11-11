/**
 * Wording for research trails (Slice 3.3, phase3-spec §5.2, §5.7): the
 * status bar's indicator, New trail, Home's note and Settings › Privacy.
 */
import type { RecordingStatus } from './api/bindings/RecordingStatus';
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
