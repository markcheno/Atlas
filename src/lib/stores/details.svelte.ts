/**
 * The document detail panel (phase1-spec §5.3): which file it shows, its
 * details, where focus goes back to when it closes, and the documents
 * shown before it via Related (Slice 2.5).
 */
import { tick } from 'svelte';
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { DocumentDetail } from '../api/bindings/DocumentDetail';

/** A document the panel showed before the current one. */
export interface PanelStep {
  fileId: string;
  title: string;
}

class DetailsState {
  fileId = $state<string | null>(null);
  detail = $state<DocumentDetail | null>(null);
  loading = $state(false);
  error = $state<AtlasApiError | null>(null);
  /** Documents shown before this one in the panel, via a related document. */
  history = $state<PanelStep[]>([]);
  /** Opened by "More like this": Related takes focus once it's shown. */
  focusRelated = $state(false);

  #seq = 0;
  #trigger: HTMLElement | null = null;

  /** Opens the panel for a file; Close returns focus to `trigger`. */
  async open(
    fileId: string,
    trigger: HTMLElement | null = null,
    options: { focusRelated?: boolean } = {},
  ) {
    this.#trigger = trigger;
    this.history = [];
    this.focusRelated = options.focusRelated ?? false;
    await this.#load(fileId);
  }

  /** Shows another file in the open panel, remembering this one for Back. */
  async show(fileId: string) {
    if (this.fileId) {
      const title = this.detail?.title ?? 'the previous document';
      this.history = [...this.history, { fileId: this.fileId, title }];
    }
    this.focusRelated = false;
    await this.#load(fileId);
  }

  /** Returns to the document shown before. */
  async back() {
    const previous = this.history.at(-1);
    if (!previous) return;
    this.history = this.history.slice(0, -1);
    this.focusRelated = false;
    await this.#load(previous.fileId);
  }

  async #load(fileId: string) {
    const seq = ++this.#seq;
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
    this.history = [];
    this.focusRelated = false;
  }
}

export const details = new DetailsState();
