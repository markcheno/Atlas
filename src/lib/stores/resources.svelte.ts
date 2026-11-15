/**
 * What background work may do now, the schedule in force and full speed
 * (Slice 4.6), kept current by the Rust governor's `resource-status`
 * events. Read by the status bar and Settings › Performance & Battery.
 */
import { isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { ResourceStatus } from '../api/bindings/ResourceStatus';

export const RESOURCE_EVENT = 'resource-status';

class ResourcesStore {
  status = $state<ResourceStatus | null>(null);
  /** What Full speed or Stop failed with, until the next try. */
  error = $state<AtlasApiError | null>(null);
  busy = $state(false);
  #unlisten: UnlistenFn | null = null;

  clear() {
    this.status = null;
    this.error = null;
  }

  async refresh() {
    try {
      this.status = await atlas.getResourceStatus();
    } catch {
      // Older mocks and a closed library: nothing to show.
    }
  }

  /** Starts listening for changes (once per app session). */
  async subscribe() {
    if (this.#unlisten || !isTauri()) return;
    this.#unlisten = await listen<ResourceStatus>(RESOURCE_EVENT, (event) => {
      this.status = event.payload;
    });
  }

  /** "Run at full speed now" (`on`) or Stop. */
  async setFullSpeed(on: boolean) {
    this.busy = true;
    this.error = null;
    try {
      this.status = await atlas.setFullSpeed(on);
    } catch (err) {
      this.error = err instanceof AtlasApiError ? err : toApiError('set_full_speed', err);
    } finally {
      this.busy = false;
    }
  }
}

export const resources = new ResourcesStore();
