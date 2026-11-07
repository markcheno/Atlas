/**
 * Which top-level view is showing. A router is unnecessary for this many
 * views. `loading` covers the moment between launch and learning whether a
 * data root is open; `boot.ts` decides the first real view.
 */
export type ViewId =
  'loading' | 'welcome' | 'data-location' | 'unavailable' | 'home' | 'library' | 'settings';

export type MainViewId = 'home' | 'library' | 'settings';

export const NAV_VIEWS: ReadonlyArray<{ id: MainViewId; label: string }> = [
  { id: 'home', label: 'Home' },
  { id: 'library', label: 'Library' },
  { id: 'settings', label: 'Settings' },
];

export function isMainView(id: ViewId): id is MainViewId {
  return id === 'home' || id === 'library' || id === 'settings';
}

class ViewState {
  current = $state<ViewId>('loading');

  go(next: ViewId) {
    this.current = next;
  }
}

export const view = new ViewState();
