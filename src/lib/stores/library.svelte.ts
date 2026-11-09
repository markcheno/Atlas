/**
 * Library browsing state: query and current page (Open and Reveal live in
 * the originals store).
 * Searches are debounced; live refreshes during cataloging are throttled.
 */
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { DocumentPage } from '../api/bindings/DocumentPage';
import type { DocumentQuery } from '../api/bindings/DocumentQuery';
import type { DocumentSort } from '../api/bindings/DocumentSort';
import type { ExtractionState } from '../api/bindings/ExtractionState';
import type { FileType } from '../api/bindings/FileType';
import { originals } from './originals.svelte';

export const PAGE_SIZE = 50;
export const SEARCH_DEBOUNCE_MS = 200;
export const LIVE_REFRESH_MS = 1000;

function asApiError(command: string, err: unknown): AtlasApiError {
  return err instanceof AtlasApiError ? err : toApiError(command, err);
}

class LibraryState {
  text = $state('');
  sourceRootId = $state<string | null>(null);
  fileType = $state<FileType | null>(null);
  includeMissing = $state(false);
  textStatus = $state<ExtractionState | null>(null);
  sort = $state<DocumentSort>('name_asc');
  page = $state(0);

  result = $state<DocumentPage | null>(null);
  loading = $state(false);
  error = $state<AtlasApiError | null>(null);

  #seq = 0;
  #debounce: ReturnType<typeof setTimeout> | null = null;
  #lastLive = 0;
  #liveTimer: ReturnType<typeof setTimeout> | null = null;

  query(): DocumentQuery {
    return {
      text: this.text.trim() === '' ? null : this.text,
      sourceRootId: this.sourceRootId,
      fileType: this.fileType,
      sort: this.sort,
      page: this.page,
      pageSize: PAGE_SIZE,
      includeMissing: this.includeMissing,
      textStatus: this.textStatus,
    };
  }

  async load() {
    const seq = ++this.#seq;
    this.loading = true;
    try {
      const result = await atlas.listDocuments(this.query());
      if (seq === this.#seq) {
        this.result = result;
        this.error = null;
      }
    } catch (err) {
      if (seq === this.#seq) this.error = asApiError('list_documents', err);
    } finally {
      if (seq === this.#seq) this.loading = false;
    }
  }

  /** Updates the search text and reloads after a short pause in typing. */
  setText(text: string, immediate = false) {
    this.text = text;
    this.page = 0;
    if (this.#debounce) clearTimeout(this.#debounce);
    if (immediate) {
      void this.load();
    } else {
      this.#debounce = setTimeout(() => void this.load(), SEARCH_DEBOUNCE_MS);
    }
  }

  setFilters(
    change: Partial<
      Pick<LibraryState, 'sourceRootId' | 'fileType' | 'includeMissing' | 'textStatus'>
    >,
  ) {
    Object.assign(this, change);
    this.page = 0;
    void this.load();
  }

  toggleSort(column: 'name' | 'modified') {
    if (column === 'name') {
      this.sort = this.sort === 'name_asc' ? 'name_desc' : 'name_asc';
    } else {
      this.sort = this.sort === 'modified_desc' ? 'modified_asc' : 'modified_desc';
    }
    this.page = 0;
    void this.load();
  }

  goTo(page: number) {
    this.page = Math.max(0, page);
    void this.load();
  }

  /** Called on cataloging progress; reloads at most once per second. */
  refreshLive() {
    const wait = LIVE_REFRESH_MS - (Date.now() - this.#lastLive);
    if (this.#liveTimer) return;
    this.#liveTimer = setTimeout(
      () => {
        this.#liveTimer = null;
        this.#lastLive = Date.now();
        void this.load();
      },
      Math.max(0, wait),
    );
  }

  reset() {
    this.text = '';
    this.sourceRootId = null;
    this.fileType = null;
    this.includeMissing = false;
    this.textStatus = null;
    this.sort = 'name_asc';
    this.page = 0;
    this.result = null;
    this.error = null;
    originals.clear();
  }
}

export const library = new LibraryState();
