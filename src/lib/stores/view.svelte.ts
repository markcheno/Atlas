/**
 * Which top-level view is showing. A router is unnecessary for this many
 * views. `loading` covers the moment between launch and learning whether a
 * data root is open; `boot.ts` decides the first real view.
 */
export type ViewId =
  | 'loading'
  | 'welcome'
  | 'data-location'
  | 'library-folders'
  | 'device-check'
  | 'unavailable'
  | 'home'
  | 'search'
  | 'library'
  | 'trails'
  | 'settings';

export type MainViewId = 'home' | 'search' | 'library' | 'trails' | 'settings';

export const NAV_VIEWS: ReadonlyArray<{ id: MainViewId; label: string }> = [
  { id: 'home', label: 'Home' },
  { id: 'search', label: 'Search' },
  { id: 'library', label: 'Library' },
  { id: 'trails', label: 'Trails' },
  { id: 'settings', label: 'Settings' },
];

export function isMainView(id: ViewId): id is MainViewId {
  return NAV_VIEWS.some((v) => v.id === id);
}

class ViewState {
  current = $state<ViewId>('loading');

  go(next: ViewId) {
    this.current = next;
  }
}

export const view = new ViewState();
