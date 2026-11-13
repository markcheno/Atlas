/**
 * Cataloging status for the open library, kept current by the Rust
 * worker's `inventory-status` events.
 */
import { isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { InventoryOverview } from '../api/bindings/InventoryOverview';

export const INVENTORY_EVENT = 'inventory-status';

class InventoryState {
  overview = $state<InventoryOverview | null>(null);
  error = $state<AtlasApiError | null>(null);
  /** What a Pause, Resume, Cancel or start failed with (Slice 4.5: shown, and
   *  not cleared by the status refresh that follows). */
  actionError = $state<AtlasApiError | null>(null);
  #unlisten: UnlistenFn | null = null;

  clear() {
    this.overview = null;
    this.error = null;
    this.actionError = null;
  }

  async refresh() {
    try {
      this.overview = await atlas.getInventoryStatus();
      this.error = null;
    } catch (err) {
      this.error = err instanceof AtlasApiError ? err : toApiError('get_inventory_status', err);
    }
  }

  /** Starts listening for progress events (once per app session). */
  async subscribe() {
    if (this.#unlisten || !isTauri()) return;
    this.#unlisten = await listen<InventoryOverview>(INVENTORY_EVENT, (event) => {
      this.overview = event.payload;
    });
  }

  async #act(command: string, action: () => Promise<unknown>) {
    this.actionError = null;
    try {
      await action();
    } catch (err) {
      this.actionError = err instanceof AtlasApiError ? err : toApiError(command, err);
    }
    await this.refresh();
  }

  pause(jobId: string) {
    return this.#act('pause_job', () => atlas.pauseJob(jobId));
  }

  resume(jobId: string) {
    return this.#act('resume_job', () => atlas.resumeJob(jobId));
  }

  cancel(jobId: string) {
    return this.#act('cancel_job', () => atlas.cancelJob(jobId));
  }

  rescan(sourceRootId: string) {
    return this.#act('start_inventory', () => atlas.startInventory(sourceRootId));
  }

  /** Check all folders now (Slice 4.5). */
  checkAll() {
    return this.#act('check_all_folders', () => atlas.checkAllFolders());
  }
}

export const inventory = new InventoryState();
