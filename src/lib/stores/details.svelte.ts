/**
 * The document detail panel (phase1-spec §5.3): which file it shows, its
 * details, where focus goes back to when it closes, and the documents
 * shown before it via Related (Slice 2.5), and the passage an Ask Atlas
 * citation pointed at (Slice 2.7).
 */
import { tick } from 'svelte';
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { DocumentDetail } from '../api/bindings/DocumentDetail';
import type { OpenVia } from '../api/bindings/OpenVia';
import type { SourceUsed } from '../api/bindings/SourceUsed';
import { FROM_RELATED } from '../openVia';
import { recordOpen } from './trails.svelte';

/** A document the panel showed before the current one. */
export interface PanelStep {
  fileId: string;
  title: string;
  via: OpenVia | null;
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
  /** Opened from an Ask Atlas citation: the passage sent to the model. */
  cited = $state<SourceUsed | null>(null);
  /** Under the cited passage: when it was sent (a trail's question). */
  citedNote = $state<string | null>(null);
  /**
   * How the user reached this document, for their trail (Slice 3.3): the
   * panel's own Open and Show use it too. None from the Library.
   */
  via = $state<OpenVia | null>(null);

  #seq = 0;
  #trigger: HTMLElement | null = null;

  /** Opens the panel for a file; Close returns focus to `trigger`. */
  async open(
    fileId: string,
    trigger: HTMLElement | null = null,
    options: {
      focusRelated?: boolean;
      cited?: SourceUsed;
      citedNote?: string;
      via?: OpenVia;
    } = {},
  ) {
    this.#trigger = trigger;
    this.history = [];
    this.focusRelated = options.focusRelated ?? false;
    this.cited = options.cited ?? null;
    this.citedNote = options.citedNote ?? null;
    this.via = options.via ?? null;
    await this.#load(fileId, true);
  }

  /** Shows a related file in the open panel, remembering this one for Back. */
  async show(fileId: string) {
    if (this.fileId) {
      const title = this.detail?.title ?? 'the previous document';
      this.history = [...this.history, { fileId: this.fileId, title, via: this.via }];
    }
    this.focusRelated = false;
    this.cited = null;
    this.citedNote = null;
    this.via = FROM_RELATED;
    await this.#load(fileId, true);
  }

  /** Returns to the document shown before; going back isn't a new step. */
  async back() {
    const previous = this.history.at(-1);
    if (!previous) return;
    this.history = this.history.slice(0, -1);
    this.focusRelated = false;
    this.cited = null;
    this.citedNote = null;
    this.via = previous.via;
    await this.#load(previous.fileId, false);
  }

  async #load(fileId: string, record: boolean) {
    const seq = ++this.#seq;
    this.fileId = fileId;
    this.detail = null;
    this.error = null;
    this.loading = true;
    try {
      const detail = await atlas.getDocumentDetail(fileId);
      if (seq === this.#seq) {
        this.detail = detail;
        if (record) recordOpen(fileId, this.via ?? undefined, 'details');
      }
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
    this.cited = null;
    this.citedNote = null;
    this.via = null;
  }
}

export const details = new DetailsState();
