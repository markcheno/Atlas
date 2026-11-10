/**
 * The open library's preferences (`config/settings.json`), shared by every
 * screen that changes one, so saving one choice never overwrites another
 * with a stale copy.
 */
import { atlas } from '../api/client';
import type { Settings } from '../api/bindings/Settings';

export const DEFAULT_SETTINGS: Settings = {
  appearance: { theme: 'system' },
  privacy: { allowRemoteEmbedding: false },
};

class SettingsStore {
  current = $state<Settings>(structuredClone(DEFAULT_SETTINGS));
  loaded = $state(false);

  async load(): Promise<Settings | null> {
    try {
      const got = await atlas.getSettings();
      this.current = {
        ...DEFAULT_SETTINGS,
        ...got,
        privacy: { ...DEFAULT_SETTINGS.privacy, ...got.privacy },
      };
      this.loaded = true;
      return this.current;
    } catch {
      this.loaded = false;
      return null;
    }
  }

  /** Saves `change` applied to the latest settings. Throws on failure. */
  async update(change: (s: Settings) => Settings): Promise<Settings> {
    const saved = await atlas.updateSettings(
      change(structuredClone($state.snapshot(this.current))),
    );
    this.current = saved;
    return saved;
  }
}

export const settings = new SettingsStore();
