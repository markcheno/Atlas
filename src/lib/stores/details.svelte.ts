/**
 * The document detail panel (phase1-spec §5.3): which file it shows, its
 * details, and where focus goes back to when it closes.
 */
import { tick } from 'svelte';
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { DocumentDetail } from '../api/bindings/DocumentDetail';

class DetailsState {
  fileId = $state<string | null>(null);
  detail = $state<DocumentDetail | null>(null);
  loading = $state(false);
  error = $state<AtlasApiError | null>(null);

  #seq = 0;
  #trigger: HTMLElement | null = null;

  async open(fileId: string, trigger: HTMLElement | null = null) {
    const seq = ++this.#seq;
    this.#trigger = trigger;
    this.fileId = fileId;
    this.detail = null;
    this.error = null;
    this.loading = true;
    try {
      const detail = await atlas.getDocumentDetail(fileId);
      if (seq === this.#seq) this.detail = detail;
    } catch (err) {
      if (seq === this.#seq) {
        this.error = err instanceof AtlasApiError ? err : toApiError('get_document_detail', err);
      }
    } finally {
      if (seq === this.#seq) this.loading = false;
    }
  }

  async close() {
    const trigger = this.#trigger;
    this.reset();
    await tick();
    if (trigger?.isConnected) trigger.focus();
  }

  reset() {
    this.#seq++;
    this.#trigger = null;
    this.fileId = null;
    this.detail = null;
    this.loading = false;
    this.error = null;
  }
}

export const details = new DetailsState();
