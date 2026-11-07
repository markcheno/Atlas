/** Saved AI service connections and their latest test results. */
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { ConnectionResult } from '../api/bindings/ConnectionResult';
import type { LocalProviderSuggestion } from '../api/bindings/LocalProviderSuggestion';
import type { ProviderProfile } from '../api/bindings/ProviderProfile';
import type { ProviderProfileInput } from '../api/bindings/ProviderProfileInput';

function asApiError(command: string, err: unknown): AtlasApiError {
  return err instanceof AtlasApiError ? err : toApiError(command, err);
}

class ProvidersState {
  profiles = $state<ProviderProfile[]>([]);
  /** Last test result per profile ID (this session only). */
  results = $state<Record<string, ConnectionResult>>({});
  testing = $state<string | null>(null);
  suggestions = $state<LocalProviderSuggestion[] | null>(null);
  detecting = $state(false);
  error = $state<AtlasApiError | null>(null);

  async refresh() {
    try {
      this.profiles = await atlas.listProviderProfiles();
      this.error = null;
    } catch (err) {
      this.error = asApiError('list_provider_profiles', err);
    }
  }

  async save(input: ProviderProfileInput): Promise<ProviderProfile | null> {
    this.error = null;
    try {
      const saved = await atlas.saveProviderProfile(input);
      await this.refresh();
      return saved;
    } catch (err) {
      this.error = asApiError('save_provider_profile', err);
      return null;
    }
  }

  async remove(id: string) {
    this.error = null;
    try {
      await atlas.deleteProviderProfile(id);
      delete this.results[id];
    } catch (err) {
      this.error = asApiError('delete_provider_profile', err);
    }
    await this.refresh();
  }

  async testSaved(id: string) {
    this.testing = id;
    try {
      this.results[id] = await atlas.testProviderConnection({
        target: 'saved_profile',
        profileId: id,
      });
    } catch (err) {
      this.error = asApiError('test_provider_connection', err);
    } finally {
      this.testing = null;
    }
  }

  /** Tests what's in the form, before or without saving. */
  async testCandidate(profile: ProviderProfileInput): Promise<ConnectionResult | null> {
    this.testing = 'candidate';
    try {
      return await atlas.testProviderConnection({ target: 'candidate', profile });
    } catch (err) {
      this.error = asApiError('test_provider_connection', err);
      return null;
    } finally {
      this.testing = null;
    }
  }

  async detect() {
    this.detecting = true;
    try {
      this.suggestions = await atlas.detectLocalProviders();
    } catch (err) {
      this.error = asApiError('detect_local_providers', err);
    } finally {
      this.detecting = false;
    }
  }

  clear() {
    this.profiles = [];
    this.results = {};
    this.suggestions = null;
    this.error = null;
  }
}

export const providers = new ProvidersState();
