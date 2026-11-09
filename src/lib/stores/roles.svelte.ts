/**
 * Model roles (Slice 2.1): which saved service and model does chat and
 * embeddings, the capability tests that justify them, and the bundled
 * recommendations.
 */
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { AiRole } from '../api/bindings/AiRole';
import type { CapabilityResult } from '../api/bindings/CapabilityResult';
import type { ModelRoles } from '../api/bindings/ModelRoles';
import type { Recommendations } from '../api/bindings/Recommendations';

function asApiError(command: string, err: unknown): AtlasApiError {
  return err instanceof AtlasApiError ? err : toApiError(command, err);
}

const EMPTY: ModelRoles = { chat: null, embedding: null };

class RolesState {
  roles = $state<ModelRoles>(EMPTY);
  recommendations = $state<Recommendations | null>(null);
  /** The last capability test per role (this session only). */
  results = $state<Partial<Record<AiRole, CapabilityResult>>>({});
  testing = $state<AiRole | null>(null);
  error = $state<AtlasApiError | null>(null);

  async refresh() {
    try {
      this.roles = await atlas.getModelRoles();
      this.error = null;
    } catch (err) {
      this.error = asApiError('get_model_roles', err);
    }
  }

  /** `listed`: model ids the user's services listed in this session. */
  async recommend(listed: string[]) {
    try {
      this.recommendations = await atlas.getModelRecommendations(listed);
    } catch (err) {
      this.error = asApiError('get_model_recommendations', err);
    }
  }

  async test(role: AiRole, profileId: string, modelId: string): Promise<CapabilityResult | null> {
    this.testing = role;
    this.error = null;
    delete this.results[role];
    try {
      const result = await atlas.testModelCapability(
        { target: 'saved_profile', profileId },
        role,
        modelId,
      );
      this.results[role] = result;
      return result;
    } catch (err) {
      this.error = asApiError('test_model_capability', err);
      return null;
    } finally {
      this.testing = null;
    }
  }

  async use(role: AiRole, profileId: string, result: CapabilityResult): Promise<boolean> {
    this.error = null;
    try {
      this.roles = await atlas.setModelRole(role, profileId, result.modelId, result);
      delete this.results[role];
      return true;
    } catch (err) {
      this.error = asApiError('set_model_role', err);
      return false;
    }
  }

  async clearRole(role: AiRole) {
    this.error = null;
    try {
      this.roles = await atlas.clearModelRole(role);
    } catch (err) {
      this.error = asApiError('clear_model_role', err);
    }
  }

  forget(role: AiRole) {
    delete this.results[role];
  }

  clear() {
    this.roles = EMPTY;
    this.recommendations = null;
    this.results = {};
    this.testing = null;
    this.error = null;
  }
}

export const roles = new RolesState();
