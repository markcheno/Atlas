/**
 * Text-reading status for the open library, kept current by the Rust
 * extraction job's `extraction-status` events (phase1-spec §5.4).
 */
import { isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { ExtractionOverview } from '../api/bindings/ExtractionOverview';

export const EXTRACTION_EVENT = 'extraction-status';

class ExtractionStore {
  overview = $state<ExtractionOverview | null>(null);
  error = $state<AtlasApiError | null>(null);
  #unlisten: UnlistenFn | null = null;

  clear() {
    this.overview = null;
    this.error = null;
  }

  async refresh() {
    try {
      // The event carries the file being read; a refresh doesn't know it,
      // so keep it while the same job is still running.
      const next = await atlas.getExtractionStatus();
      const current = this.overview?.active;
      if (next.active && current && current.jobId === next.active.jobId) {
        next.active.currentFileId ??= current.currentFileId;
      }
      this.overview = next;
      this.error = null;
    } catch (err) {
      this.error = err instanceof AtlasApiError ? err : toApiError('get_extraction_status', err);
    }
  }

  /** Starts listening for progress events (once per app session). */
  async subscribe() {
    if (this.#unlisten || !isTauri()) return;
    this.#unlisten = await listen<ExtractionOverview>(EXTRACTION_EVENT, (event) => {
      this.overview = event.payload;
    });
  }

  async #act(command: string, action: () => Promise<unknown>) {
    this.error = null;
    try {
      await action();
    } catch (err) {
      this.error = err instanceof AtlasApiError ? err : toApiError(command, err);
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

  /** "Read text now" for one folder (idempotent). */
  readNow(sourceRootId: string) {
    return this.#act('start_extraction', () => atlas.startExtraction(sourceRootId));
  }
}

export const extraction = new ExtractionStore();
