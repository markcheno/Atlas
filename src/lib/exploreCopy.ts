/**
 * Plain-language wording for Explore (Slice 4.6.1): the map's status, its
 * empty states, and what each area is called.
 */
import type { ExploreArea } from './api/bindings/ExploreArea';
import type { ExploreStatus } from './api/bindings/ExploreStatus';
import type { ExploreSubArea } from './api/bindings/ExploreSubArea';
import type { PauseReason } from './api/bindings/PauseReason';
import { count } from './inventoryCopy';

/** What an area is called: the owner's name, else its words. */
export function areaLabel(a: ExploreArea | ExploreSubArea): string {
  if (a.name) return a.name;
  return a.words.length ? a.words.join(' · ') : `Area ${a.number}`;
}

/** "3 · Philosophy", for a filter chip. */
export function areaChip(a: ExploreArea | ExploreSubArea): string {
  return `${a.number} · ${areaLabel(a)}`;
}

function ago(iso: string, now: Date): string {
  const minutes = Math.max(0, Math.round((now.getTime() - Date.parse(iso)) / 60_000));
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${count(minutes, 'minute')} ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${count(hours, 'hour')} ago`;
  return `${count(Math.round(hours / 24), 'day')} ago`;
}

/** The line under the heading. */
export function statusLine(s: ExploreStatus | null, now = new Date()): string {
  if (!s) return '';
  if (s.state !== 'ready') return '';
  const parts = [`${count(s.books, 'book')}`];
  if (s.computedAt) parts.push(`drawn ${ago(s.computedAt, now)}`);
  if (s.building) parts.push('being redrawn');
  else if (s.newSince > 0) parts.push(`${count(s.newSince, 'book')} new or changed since`);
  return parts.join(' · ');
}

const HELD: Partial<Record<PauseReason, string>> = {
  paused_by_user: 'Background work is paused in Settings.',
  outside_hours: 'Atlas will redraw it during the hours set in Settings.',
  not_charging: 'Atlas will redraw it once this computer is plugged in.',
  battery_low: 'The battery is low.',
  low_disk_space: "The drive with Atlas's data is nearly full.",
  memory_pressure: 'The computer is short on memory.',
  thermal_pressure: 'The computer is running hot.',
};

/** Why a redraw waits. */
export function heldText(reason: PauseReason | null): string | null {
  if (!reason) return null;
  return `The redraw waits. ${HELD[reason] ?? ''}`.trim();
}

/** The view's empty states. */
export function emptyText(s: ExploreStatus | null): { title: string; body: string } | null {
  if (!s) return null;
  switch (s.state) {
    case 'no_model':
      return {
        title: 'Explore needs meaning indexing',
        body: 'Explore lays your books out by what they are about. Choose an embedding model in Settings › AI services, and Atlas draws the map once your books are indexed.',
      };
    case 'too_few':
      return {
        title: 'Not enough books yet',
        body: `A map needs at least ${count(s.minBooks, 'book')} indexed for meaning; ${s.libraryBooks.toLocaleString('en-US')} ${s.libraryBooks === 1 ? 'is' : 'are'} so far.`,
      };
    case 'not_drawn':
      return s.building
        ? {
            title: 'Drawing the map…',
            body: 'Atlas is grouping your books by meaning. This takes a few seconds for a few thousand books.',
          }
        : {
            title: 'The map is waiting',
            body: 'Atlas draws it in the background when the computer is plugged in and not busy. Draw it now to start.',
          };
    default:
      return null;
  }
}

/** After "Find on the map". */
export function foundText(found: number, onMap: number): string {
  if (found === 0) return 'Nothing found.';
  return `${count(onMap, 'result')} of ${found.toLocaleString('en-US')} on the map`;
}
