/**
 * Appearance preference. "system" follows the OS light/dark setting via
 * `prefers-color-scheme`; "light" and "dark" force a theme through the
 * `data-theme` attribute on <html>.
 *
 * Held in memory in Slice 0.1. It will persist to `config/settings.json`
 * inside the Atlas data root from Slice 0.2; browser storage is not used
 * because Atlas-owned persistent data belongs under that one root.
 */
export type ThemePreference = 'system' | 'light' | 'dark';

export const THEME_OPTIONS: ReadonlyArray<{ value: ThemePreference; label: string }> = [
  { value: 'system', label: 'Match system' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
];

class ThemeState {
  preference = $state<ThemePreference>('system');

  set(next: ThemePreference) {
    this.preference = next;
    const root = document.documentElement;
    if (next === 'system') {
      delete root.dataset.theme;
    } else {
      root.dataset.theme = next;
    }
  }
}

export const theme = new ThemeState();
