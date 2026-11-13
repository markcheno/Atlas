/**
 * Meaning-index status for the open library, kept current by the Rust
 * embedding job's `embedding-status` events (Slice 2.3, phase2-spec §5.2).
 */
import { isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { EmbeddingOverview } from '../api/bindings/EmbeddingOverview';

export const EMBEDDING_EVENT = 'embedding-status';

class EmbeddingStore {
  overview = $state<EmbeddingOverview | null>(null);
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
      this.overview = await atlas.getEmbeddingStatus();
      this.error = null;
    } catch (err) {
      this.error = err instanceof AtlasApiError ? err : toApiError('get_embedding_status', err);
    }
  }

  /** Starts listening for progress events (once per app session). */
  async subscribe() {
    if (this.#unlisten || !isTauri()) return;
    this.#unlisten = await listen<EmbeddingOverview>(EMBEDDING_EVENT, (event) => {
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

  /** Resume, and Retry after the service was unreachable. */
  resume(jobId: string) {
    return this.#act('resume_job', () => atlas.resumeJob(jobId));
  }

  cancel(jobId: string) {
    return this.#act('cancel_job', () => atlas.cancelJob(jobId));
  }

  /** "Index meaning now" for one folder (idempotent; retries a failure). */
  indexNow(sourceRootId: string) {
    return this.#act('start_embedding', () => atlas.startEmbedding(sourceRootId));
  }
}

export const embedding = new EmbeddingStore();
