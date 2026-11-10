/**
 * Appearance preference. "system" follows the OS light/dark setting via
 * `prefers-color-scheme`; "light" and "dark" force a theme through the
 * `data-theme` attribute on <html>.
 *
 * Saved in `config/settings.json` inside the Atlas data root. Browser
 * storage is not used: Atlas-owned persistent data belongs under that root.
 * Before a root is open the choice lasts only for the session.
 */
import type { AtlasApiError } from '../api/client';
import type { ThemePreference } from '../api/bindings/ThemePreference';
import { settings } from './settings.svelte';

export type { ThemePreference };

export const THEME_OPTIONS: ReadonlyArray<{ value: ThemePreference; label: string }> = [
  { value: 'system', label: 'Match system' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
];

class ThemeState {
  preference = $state<ThemePreference>('system');
  saveError = $state<AtlasApiError | null>(null);

  /** Applies a preference to the document without saving it. */
  apply(next: ThemePreference) {
    this.preference = next;
    const root = document.documentElement;
    if (next === 'system') {
      delete root.dataset.theme;
    } else {
      root.dataset.theme = next;
    }
  }

  /** Loads the saved preference from the open data root. */
  async load() {
    const loaded = await settings.load();
    // No open root: keep the session preference.
    if (loaded) this.apply(loaded.appearance.theme);
  }

  /** Applies and saves a preference chosen by the user. */
  async choose(next: ThemePreference, persist: boolean) {
    this.apply(next);
    this.saveError = null;
    if (!persist) return;
    try {
      await settings.update((s) => ({ ...s, appearance: { ...s.appearance, theme: next } }));
    } catch (err) {
      this.saveError = err as AtlasApiError;
    }
  }
}

export const theme = new ThemeState();
