/**
 * Appearance preference. "system" follows the OS light/dark setting via
 * `prefers-color-scheme`; "light" and "dark" force a theme through the
 * `data-theme` attribute on <html>.
 *
 * Saved in `config/settings.json` inside the Atlas data root. Browser
 * storage is not used: Atlas-owned persistent data belongs under that root.
 * Before a root is open the choice lasts only for the session.
 */
import { atlas, type AtlasApiError } from '../api/client';
import type { Settings } from '../api/bindings/Settings';
import type { ThemePreference } from '../api/bindings/ThemePreference';

export type { ThemePreference };

export const THEME_OPTIONS: ReadonlyArray<{ value: ThemePreference; label: string }> = [
  { value: 'system', label: 'Match system' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
];

class ThemeState {
  preference = $state<ThemePreference>('system');
  saveError = $state<AtlasApiError | null>(null);
  #settings: Settings = { appearance: { theme: 'system' } };

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
    try {
      this.#settings = await atlas.getSettings();
      this.apply(this.#settings.appearance.theme);
    } catch {
      // No open root: keep the session preference.
    }
  }

  /** Applies and saves a preference chosen by the user. */
  async choose(next: ThemePreference, persist: boolean) {
    this.apply(next);
    this.saveError = null;
    if (!persist) return;
    const updated: Settings = {
      ...this.#settings,
      appearance: { ...this.#settings.appearance, theme: next },
    };
    try {
      this.#settings = await atlas.updateSettings(updated);
    } catch (err) {
      this.saveError = err as AtlasApiError;
    }
  }
}

export const theme = new ThemeState();
