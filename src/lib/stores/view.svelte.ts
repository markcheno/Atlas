/**
 * Which top-level view is showing. A router is unnecessary for four views.
 *
 * Welcome shows on every launch in Slice 0.1 because "first run" can only
 * be remembered once the Atlas data root exists (Slice 0.2).
 */
export type ViewId = 'welcome' | 'home' | 'library' | 'settings';

export const NAV_VIEWS: ReadonlyArray<{ id: Exclude<ViewId, 'welcome'>; label: string }> = [
  { id: 'home', label: 'Home' },
  { id: 'library', label: 'Library' },
  { id: 'settings', label: 'Settings' },
];

class ViewState {
  current = $state<ViewId>('welcome');

  go(next: ViewId) {
    this.current = next;
  }
}

export const view = new ViewState();
